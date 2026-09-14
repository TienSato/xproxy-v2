//! Test cho câu hỏi "chạy nhiều cổng thì có cổng nào hút hết của cổng khác không?".
//!
//!     cargo test -p xproxy-core --test fairness

mod common;

use std::time::{Duration, Instant};

use common::{round_trip, socks_connect_domain, FakeUpstream, Origin, CLOSED};
use xproxy_core::relay::{GlobalLimit, SlotConfig, SlotHandle, Upstream};


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

/// Một cổng chạm trần kết nối của NÓ thì các cổng khác vẫn phục vụ bình thường.
///
/// Đây là cái quan trọng nhất: nếu không có trần riêng từng cổng, một thiết bị mở
/// hàng nghìn kết nối sẽ làm cạn file descriptor của cả tiến trình và mọi cổng khác
/// cùng chết — biểu hiện đúng như "cổng này hút hết của cổng kia".
#[tokio::test]
async fn cong_cham_tran_khong_lam_chet_cong_khac() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;
    let global = GlobalLimit::new(10_000);

    // Cổng tham lam: chỉ cho 2 kết nối.
    let greedy = SlotHandle::bind(
        0,
        SlotConfig { max_conns: Some(2), ..cfg() },
        global.clone(),
    )
    .await
    .unwrap();
    // Cổng hàng xóm: bình thường.
    let neighbour = SlotHandle::bind(0, cfg(), global.clone()).await.unwrap();

    let u = Upstream::new("127.0.0.1", up.addr.port());
    greedy.set_upstream(u.clone()).await;
    neighbour.set_upstream(u).await;

    // Giữ 2 kết nối trên cổng tham lam (chưa gửi gì nên chúng còn mở).
    let mut held = Vec::new();
    for _ in 0..2 {
        held.push(
            socks_connect_domain(greedy.port(), "127.0.0.1", origin.addr.port())
                .await
                .unwrap(),
        );
    }
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Kết nối thứ 3 vào cổng tham lam bị từ chối…
    let refused = socks_connect_domain(greedy.port(), "127.0.0.1", origin.addr.port()).await;
    assert_eq!(
        refused.err().map(|e| e.code),
        Some(CLOSED),
        "kết nối vượt trần phải bị đóng thẳng"
    );

    // …nhưng cổng hàng xóm vẫn chạy ngon lành.
    let s = socks_connect_domain(neighbour.port(), "127.0.0.1", origin.addr.port())
        .await
        .expect("cổng khác KHÔNG được bị ảnh hưởng");
    assert_eq!(round_trip(s, b"toi van song").await, "[A]toi van song");

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(greedy.stats().rejected >= 1);
    assert_eq!(neighbour.stats().rejected, 0);
    drop(held);
}

/// Chế độ TỰ ĐỘNG: máy còn rảnh thì không chặn ai, căng rồi mới ép về phần chia đều.
///
/// Đây là hành vi mặc định — người dùng không phải đặt con số nào.
#[tokio::test]
async fn tu_dong_chia_deu_khi_may_cang() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;

    // Trần chung 40, hai cổng → phần chia đều = 20. Ngưỡng "căng" = 60% × 40 = 24.
    let global = GlobalLimit::new(40);
    let a = SlotHandle::bind(0, cfg(), global.clone()).await.unwrap();
    let b = SlotHandle::bind(0, cfg(), global.clone()).await.unwrap();
    let u = Upstream::new("127.0.0.1", up.addr.port());
    a.set_upstream(u.clone()).await;
    b.set_upstream(u).await;

    assert_eq!(global.fair_share(), 20, "40 chỗ chia cho 2 cổng");

    // Cổng A mở dần. Trong lúc máy còn rảnh (< 24 kết nối) nó được vượt xa phần chia
    // đều của mình — đúng ý đồ: một cổng chạy một mình thì dùng hết công suất.
    let mut held = Vec::new();
    for _ in 0..24 {
        match socks_connect_domain(a.port(), "127.0.0.1", origin.addr.port()).await {
            Ok(s) => held.push(s),
            Err(_) => break,
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        held.len() > 20,
        "lúc máy rảnh cổng A phải được vượt phần chia đều, mới mở được {}",
        held.len()
    );

    // Giờ đã căng → A bị ép về phần chia đều, không mở thêm được.
    assert!(
        socks_connect_domain(a.port(), "127.0.0.1", origin.addr.port())
            .await
            .is_err(),
        "đã căng thì cổng A phải bị ghìm lại"
    );

    // Nhưng B chưa dùng gì, vẫn phải vào được.
    let s = socks_connect_domain(b.port(), "127.0.0.1", origin.addr.port())
        .await
        .expect("cổng chưa dùng gì phải luôn có chỗ");
    assert_eq!(round_trip(s, b"toi moi den").await, "[A]toi moi den");

    drop(held);
}

/// Trần chung của cả app cũng phải chặn được, vì file descriptor là tài nguyên chung.
#[tokio::test]
async fn tran_chung_cua_ca_app() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;
    let global = GlobalLimit::new(2); // cả app chỉ 2 kết nối

    let a = SlotHandle::bind(0, cfg(), global.clone()).await.unwrap();
    let b = SlotHandle::bind(0, cfg(), global.clone()).await.unwrap();
    let u = Upstream::new("127.0.0.1", up.addr.port());
    a.set_upstream(u.clone()).await;
    b.set_upstream(u).await;

    let _h1 = socks_connect_domain(a.port(), "127.0.0.1", origin.addr.port())
        .await
        .unwrap();
    let _h2 = socks_connect_domain(a.port(), "127.0.0.1", origin.addr.port())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(global.open(), 2);

    // Cổng b xin kết nối thứ 3 của cả app → bị từ chối.
    assert!(
        socks_connect_domain(b.port(), "127.0.0.1", origin.addr.port())
            .await
            .is_err()
    );
}

/// Mặc định KHÔNG giới hạn băng thông: nhiều cổng chạy song song thì tổng thời gian
/// phải xấp xỉ thời gian một cổng, chứ không cộng dồn (tức là chúng thật sự chạy
/// đồng thời, không xếp hàng sau nhau).
#[tokio::test]
async fn cac_cong_truyen_song_song_chu_khong_xep_hang() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;
    let global = GlobalLimit::new(10_000);

    let mut slots = Vec::new();
    for _ in 0..8 {
        let s = SlotHandle::bind(0, cfg(), global.clone()).await.unwrap();
        s.set_upstream(Upstream::new("127.0.0.1", up.addr.port())).await;
        slots.push(s);
    }

    let payload = vec![b'x'; 256 * 1024];
    let oport = origin.addr.port();

    // Một cổng chạy một mình, lấy làm mốc.
    let t0 = Instant::now();
    {
        let s = socks_connect_domain(slots[0].port(), "127.0.0.1", oport).await.unwrap();
        let got = round_trip(s, &payload).await;
        assert_eq!(got.len(), payload.len() + 3);
    }
    let one = t0.elapsed();

    // Tám cổng chạy cùng lúc.
    let t1 = Instant::now();
    let mut tasks = Vec::new();
    for s in &slots {
        let port = s.port();
        let p = payload.clone();
        tasks.push(tokio::spawn(async move {
            let c = socks_connect_domain(port, "127.0.0.1", oport).await.unwrap();
            round_trip(c, &p).await.len()
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), payload.len() + 3);
    }
    let eight = t1.elapsed();

    // Nếu bị xếp hàng thì eight ≈ 8 × one. Cho biên rộng vì máy CI hay nhiễu.
    assert!(
        eight < one * 5 + Duration::from_millis(500),
        "8 cổng song song mất {eight:?}, một cổng mất {one:?} — có vẻ đang xếp hàng"
    );
}

/// Đặt trần băng thông cho một cổng thì cổng đó chậm lại, cổng kia KHÔNG bị ảnh hưởng.
#[tokio::test]
async fn ghim_bang_thong_mot_cong_khong_anh_huong_cong_khac() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;
    let global = GlobalLimit::new(10_000);

    // 128 KB/s — truyền 512KB sẽ mất khoảng 2 giây (gáo đầy sẵn 256KB).
    let slow = SlotHandle::bind(
        0,
        SlotConfig { rate_limit_bps: 128 * 1024, ..cfg() },
        global.clone(),
    )
    .await
    .unwrap();
    let fast = SlotHandle::bind(0, cfg(), global.clone()).await.unwrap();

    let u = Upstream::new("127.0.0.1", up.addr.port());
    slow.set_upstream(u.clone()).await;
    fast.set_upstream(u).await;

    let payload = vec![b'y'; 512 * 1024];
    let oport = origin.addr.port();

    let p1 = payload.clone();
    let slow_port = slow.port();
    let slow_task = tokio::spawn(async move {
        let t = Instant::now();
        let c = socks_connect_domain(slow_port, "127.0.0.1", oport).await.unwrap();
        round_trip(c, &p1).await;
        t.elapsed()
    });

    // Cổng nhanh chạy trong lúc cổng chậm đang bị ghìm.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let t = Instant::now();
    let c = socks_connect_domain(fast.port(), "127.0.0.1", oport).await.unwrap();
    let got = round_trip(c, &payload).await;
    let fast_time = t.elapsed();
    assert_eq!(got.len(), payload.len() + 3);

    let slow_time = slow_task.await.unwrap();

    assert!(
        fast_time < Duration::from_millis(800),
        "cổng không giới hạn bị chậm lây: {fast_time:?}"
    );
    assert!(
        slow_time > fast_time,
        "cổng bị ghìm ({slow_time:?}) phải chậm hơn cổng thường ({fast_time:?})"
    );
}

/// Không đặt trần thì không được có chi phí thừa: 200 kết nối liên tiếp vẫn nhanh.
#[tokio::test]
async fn khong_dat_tran_thi_khong_ton_chi_phi() {
    let origin = Origin::start("[A]").await;
    let up = FakeUpstream::start(None).await;
    let slot = SlotHandle::bind(0, cfg(), GlobalLimit::new(10_000)).await.unwrap();
    slot.set_upstream(Upstream::new("127.0.0.1", up.addr.port())).await;

    let t = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..200 {
        let port = slot.port();
        let oport = origin.addr.port();
        tasks.push(tokio::spawn(async move {
            let c = socks_connect_domain(port, "127.0.0.1", oport).await.unwrap();
            round_trip(c, b"ping").await
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), "[A]ping");
    }
    assert!(t.elapsed() < Duration::from_secs(10), "quá chậm: {:?}", t.elapsed());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(slot.stats().total, 200);
    assert_eq!(slot.stats().open, 0);
}
