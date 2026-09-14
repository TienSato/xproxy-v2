//! Nâng giới hạn file descriptor lúc khởi động.
//!
//! Đây là một cái bẫy thật trên macOS: app chạy từ Finder có `RLIMIT_NOFILE` mềm mặc
//! định chỉ **256**. Mỗi kết nối proxy tốn 2 fd (một phía thiết bị, một phía upstream),
//! nên 50 cổng đang tải là chạm trần rất nhanh. Khi chạm trần, `accept()` của MỌI cổng
//! bắt đầu lỗi `EMFILE` — biểu hiện đúng như "một cổng làm chết các cổng khác", dù
//! chẳng liên quan gì tới băng thông.
//!
//! Windows không có khái niệm này (giới hạn handle rất cao), nên hàm chỉ là no-op.

/// Nâng giới hạn mềm lên hết mức cho phép. Trả về giới hạn sau khi nâng.
#[cfg(unix)]
pub fn raise_fd_limit() -> u64 {
    unsafe {
        let mut lim: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) != 0 {
            return 0;
        }

        let cur = lim.rlim_cur;

        // macOS hay báo rlim_max = RLIM_INFINITY nhưng nhân thật ra chặn ở
        // kern.maxfilesperproc. Đặt thẳng vô cực sẽ bị từ chối, nên thử lùi dần.
        for candidate in [lim.rlim_max, 65536, 10240, 4096] {
            if candidate <= cur {
                break; // danh sách giảm dần: đã thấp hơn mức hiện tại thì thôi
            }
            lim.rlim_cur = candidate;
            if libc::setrlimit(libc::RLIMIT_NOFILE, &lim) == 0 {
                break;
            }
        }

        let mut now: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut now) == 0 {
            now.rlim_cur as u64
        } else {
            cur as u64
        }
    }
}

#[cfg(not(unix))]
pub fn raise_fd_limit() -> u64 {
    // Windows: handle không bị giới hạn kiểu này.
    u64::MAX
}

/// Số kết nối đồng thời an toàn suy ra từ giới hạn fd hiện tại.
///
/// Mỗi kết nối tốn 2 fd; chừa lại 25% cho listener, file cấu hình, kết nối HTTPS gọi
/// API… nên chỉ dùng 3/4 hạn mức.
pub fn safe_global_conns() -> usize {
    let fds = raise_fd_limit();
    if fds == u64::MAX {
        return 8192;
    }
    let usable = fds.saturating_sub(128) * 3 / 4 / 2;
    usable.clamp(64, 16384) as usize
}
