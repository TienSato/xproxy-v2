//! Test tích hợp cho relay — phần thay gost.
//!
//! Chạy hoàn toàn trên loopback: không cần mạng, không cần tài khoản nhà cung cấp.
//!
//!     cargo test -p xproxy-core

mod common;

use std::time::Duration;

use common::{round_trip, socks_connect_domain, FakeUpstream, Origin};
use xproxy_core::relay::{GlobalLimit, Scheme, SlotConfig, SlotHandle, Upstream};


/// Nâng giới hạn file descriptor cho tiến trình test.
///
/// App thật làm việc này trong `Manager::new()`, nhưng test gọi thẳng `SlotHandle::bind`
/// nên phải tự gọi. Không có nó thì Terminal macOS chỉ cho 256 fd — mở vài trăm kết nối
/// là `accept()` bắt đầu trả EMFILE và test đỏ vì đúng cái bẫy mà code này sinh ra để
/// tránh.
fn ensure_fds() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let n = xproxy_core::rlimit::raise_fd_limit();
        eprintln!("giới hạn fd của tiến trình test: {n}");
    });
}

fn cfg() -> SlotConfig {
    ensure_fds();
    SlotConfig {
        dial_timeout: Duration::from_secs(5),
        ..Default::default()
    }
}

/// Mở một cổng với hạn mức chung rộng rãi (test không quan tâm trần toàn app).
async fn bind(config: SlotConfig) -> SlotHandle {
    SlotHandle::bind(0, config, GlobalLimit::new(10_000))
        .await
        .unwrap()
}

/// Đường đi đầy đủ: thiết bị → relay → upstream (có user/pass) → server đích.
#[tokio::test]
async fn di_qua_relay_toi_dich() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(Some(("bob", "s3cret"))).await;

    let slot = bind(cfg()).await;
    slot.set_upstream(
        Upstream::new(up.addr.ip().to_string(), up.addr.port()).with_auth("bob", "s3cret"),
    )
    .await;

    let s = socks_connect_domain(slot.port(), "127.0.0.1", origin.addr.port())
        .await
        .expect("relay phải trả lời thành công");
    let got = round_trip(s, b"xin chao").await;

    assert_eq!(got, "[A]xin chao");
    assert_eq!(up.hits.load(std::sync::atomic::Ordering::Relaxed), 1);
}

/// Tên miền phải được CHUYỂN NGUYÊN cho upstream, không resolve tại máy này.
/// Nếu resolve ở đây thì DNS đi ra từ IP thật → lộ vị trí dù traffic qua proxy.
#[tokio::test]
async fn khong_resolve_dns_tai_may() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;

    let slot = bind(cfg()).await;
    slot.set_upstream(Upstream::new(up.addr.ip().to_string(), up.addr.port()))
        .await;

    // "localhost" là tên miền; upstream phải nhận đúng chuỗi đó.
    let s = socks_connect_domain(slot.port(), "localhost", origin.addr.port())
        .await
        .unwrap();
    let _ = round_trip(s, b"x").await;

    assert_eq!(up.last_domain.lock().await.as_deref(), Some("localhost"));
}

/// **Test quan trọng nhất.** Đổi upstream giữa chừng:
/// - số cổng KHÔNG đổi
/// - không có khoảng chết: kết nối ngay sau khi đổi đã đi đường mới
///
/// Bản Swift phải giết tiến trình gost rồi bind lại, nên mới sinh ra
/// `terminateAndWait` + `lsof` + `kill -9`. Ở đây chỉ là một lần ghi vào RwLock.
#[tokio::test]
async fn doi_upstream_khong_dong_cong() {
    let origin = Origin::start("[A]").await;
    let up_a = FakeUpstream::start(None).await;
    let up_b = FakeUpstream::start(None).await;

    let slot = bind(cfg()).await;
    let port_truoc = slot.port();

    slot.set_upstream(Upstream::new("127.0.0.1", up_a.addr.port())).await;
    let s = socks_connect_domain(port_truoc, "127.0.0.1", origin.addr.port())
        .await
        .unwrap();
    assert_eq!(round_trip(s, b"1").await, "[A]1");

    // Đổi IP.
    slot.set_upstream(Upstream::new("127.0.0.1", up_b.addr.port())).await;

    let port_sau = slot.port();
    assert_eq!(port_truoc, port_sau, "số cổng phải giữ nguyên sau khi Đổi");

    let s = socks_connect_domain(port_sau, "127.0.0.1", origin.addr.port())
        .await
        .expect("cổng phải nhận kết nối NGAY, không có khoảng chết");
    assert_eq!(round_trip(s, b"2").await, "[A]2");

    use std::sync::atomic::Ordering::Relaxed;
    assert_eq!(up_a.hits.load(Relaxed), 1, "kết nối đầu đi đường A");
    assert_eq!(up_b.hits.load(Relaxed), 1, "kết nối sau đi đường B");
}

/// Đếm băng thông — thứ bản Swift không làm được vì byte đi qua tiến trình gost.
#[tokio::test]
async fn dem_bang_thong() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;

    let slot = bind(cfg()).await;
    slot.set_upstream(Upstream::new("127.0.0.1", up.addr.port())).await;

    let payload = vec![b'z'; 5000];
    let s = socks_connect_domain(slot.port(), "127.0.0.1", origin.addr.port())
        .await
        .unwrap();
    let got = round_trip(s, &payload).await;
    assert_eq!(got.len(), 5003); // "[A]" + payload

    // Chờ task phục vụ kết nối chạy nốt.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let st = slot.stats();
    assert_eq!(st.up, 5000, "byte gửi lên phải đúng bằng payload");
    assert_eq!(st.down, 5003, "byte nhận về phải đúng bằng phản hồi");
    assert_eq!(st.total, 1);
    assert_eq!(st.open, 0, "kết nối đã đóng thì không còn đếm là đang mở");
}

/// Cổng đã mở nhưng chưa gán IP: client phải nhận mã lỗi rõ ràng (0x02 = không cho
/// phép), thay vì "connection refused" khó hiểu.
#[tokio::test]
async fn chua_gan_ip_thi_tu_choi_co_ly_do() {
    let slot = bind(cfg()).await;
    let err = socks_connect_domain(slot.port(), "example.com", 80)
        .await
        .err()
        .expect("phải bị từ chối");
    assert_eq!(err.code, 0x02);
    assert_eq!(slot.stats().failed, 1);
}

/// Upstream sai mật khẩu → relay trả mã 0x05 và đếm vào `failed`.
#[tokio::test]
async fn sai_mat_khau_upstream() {
    let up = FakeUpstream::start(Some(("bob", "dung"))).await;
    let slot = bind(cfg()).await;
    slot.set_upstream(
        Upstream::new("127.0.0.1", up.addr.port()).with_auth("bob", "sai"),
    )
    .await;

    let err = socks_connect_domain(slot.port(), "example.com", 80)
        .await
        .err()
        .expect("sai mật khẩu thì phải lỗi");
    assert_eq!(err.code, 0x05);
}

/// Gỡ cổng phải nhả socket NGAY — không cần poll đợi, không cần `lsof`/`kill -9`.
#[tokio::test]
async fn go_cong_nha_socket_ngay() {
    let slot = bind(cfg()).await;
    let port = slot.port();

    // Đang giữ thì không bind lại được.
    assert!(tokio::net::TcpListener::bind(("127.0.0.1", port)).await.is_err());

    slot.shutdown();

    // Nhả trong vòng 500ms (thực tế là ngay lập tức; để dư cho máy CI chậm).
    let mut freed = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        if tokio::net::TcpListener::bind(("0.0.0.0", port)).await.is_ok() {
            freed = true;
            break;
        }
    }
    assert!(freed, "cổng phải được nhả sau khi shutdown");
}

/// Bật `drop_on_rotate`: đổi IP thì ngắt luôn kết nối đang mở.
#[tokio::test]
async fn drop_on_rotate_ngat_ket_noi_cu() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;

    let slot = bind(SlotConfig {
        drop_on_rotate: true,
        ..cfg()
    })
    .await;
    slot.set_upstream(Upstream::new("127.0.0.1", up.addr.port())).await;

    let mut s = socks_connect_domain(slot.port(), "127.0.0.1", origin.addr.port())
        .await
        .unwrap();
    s.write_all(b"giu ket noi").await.unwrap();

    // Đổi IP → kết nối đang mở bị ngắt.
    slot.set_upstream(Upstream::new("127.0.0.1", up.addr.port())).await;

    let mut buf = [0u8; 16];
    let r = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf)).await;
    assert!(
        matches!(r, Ok(Ok(0)) | Ok(Err(_))),
        "kết nối cũ phải bị ngắt khi drop_on_rotate = true"
    );
}

/// Cổng ở chế độ HTTP: `CONNECT host:port` phải tunnel được qua upstream SOCKS5.
#[tokio::test]
async fn che_do_http_connect() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;

    let slot = bind(SlotConfig {
        listen_scheme: Scheme::Http,
        ..cfg()
    })
    .await;
    slot.set_upstream(Upstream::new("127.0.0.1", up.addr.port())).await;

    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", slot.port()))
        .await
        .unwrap();
    let req = format!(
        "CONNECT 127.0.0.1:{p} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n",
        p = origin.addr.port()
    );
    s.write_all(req.as_bytes()).await.unwrap();

    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while head.len() < 4 || &head[head.len() - 4..] != b"\r\n\r\n" {
        s.read_exact(&mut b).await.unwrap();
        head.push(b[0]);
    }
    assert!(
        String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"),
        "phải trả 200 Connection Established, nhận được: {}",
        String::from_utf8_lossy(&head)
    );

    s.write_all(b"qua tunnel").await.unwrap();
    s.shutdown().await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert_eq!(out, "[A]qua tunnel");
}

/// Nhiều kết nối cùng lúc trên một cổng.
#[tokio::test]
async fn nhieu_ket_noi_song_song() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;

    let slot = bind(cfg()).await;
    slot.set_upstream(Upstream::new("127.0.0.1", up.addr.port())).await;

    let port = slot.port();
    let oport = origin.addr.port();
    let mut tasks = Vec::new();
    for i in 0..20u32 {
        tasks.push(tokio::spawn(async move {
            let s = socks_connect_domain(port, "127.0.0.1", oport).await.unwrap();
            round_trip(s, format!("n{i}").as_bytes()).await
        }));
    }
    for (i, t) in tasks.into_iter().enumerate() {
        assert_eq!(t.await.unwrap(), format!("[A]n{i}"));
    }

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(slot.stats().total, 20);
    assert_eq!(slot.stats().open, 0);
}
