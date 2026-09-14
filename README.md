# xproxy 2

Bản viết lại của xproxy bằng **Rust + Tauri 2**, chạy cả macOS và Windows.

Khác biệt lớn nhất so với bản Swift: **không còn gost**. Phần chuyển tiếp proxy nằm
thẳng trong app (`crates/xproxy-core/src/relay/`), nên:

| | Bản Swift + gost | Bản này |
|---|---|---|
| Tiến trình khi chạy 50 cổng | 51 (app + 50 gost, x86_64 qua Rosetta) | 1, native |
| Đổi IP | giết gost → đợi socket nhả → bind lại | ghi vào một `RwLock`, cổng không rớt |
| Dọn cổng kẹt | `lsof` + `kill -9` + `pkill` (~80 dòng) | không cần |
| Kiểm tra sống | spawn `nc` | `TcpStream::connect` |
| Lấy IP thoát | spawn `curl` | `reqwest` |
| Đếm băng thông từng cổng | không làm được | có |
| Cấu hình nhà cung cấp | 2 chế độ + 10 ô tên tham số | một ô URL |
| Windows | không | có |

---

## Chạy thử

```bash
cd /Users/sato/Documents/xproxy-2
cargo test -p xproxy-core
```

Bộ test không cần mạng, không cần tài khoản nhà cung cấp — nó tự dựng một SOCKS5
upstream giả và một server đích giả trên loopback.

---

## Cài đặt

```bash
# 1. Rust (nếu chưa có)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Phụ thuộc frontend
npm install
```

Icon đã có sẵn trong `src-tauri/icons/`. Yêu cầu cho Windows: xem [BUILD.md](BUILD.md).

## Chạy

```bash
npm run tauri dev      # chạy thử, hot-reload frontend
```

Đóng gói `.dmg` cho macOS và `.exe` cho Windows: xem **[BUILD.md](BUILD.md)**.

## Test

```bash
cargo test -p xproxy-core          # toàn bộ
cargo test -p xproxy-core --test relay -- --nocapture
```

---

## Nhà cung cấp: chỉ một ô URL

Mỗi nhà cung cấp = một cái tên + một URL. Dán nguyên URL từ trang của họ, key nằm luôn
trong URL:

```
https://webipapi.cliproxy.com/api/getIpInfo?key=...&port=443&num=1&country=US&state=&type=2
```

Mỗi lần bấm **Gán** / **Đổi**, app gọi URL đó một lần, đọc dòng trả về
(`64.205.177.44:443:6ab9171c2721211f:kd7c2apyfaxslfb61syy`) và cắm vào đúng cổng vừa bấm.
Thêm hãng mới = dán thêm một URL, không phải khai báo tên tham số như bản Swift.

App chỉ động vào URL ở ba chỗ, còn lại giữ nguyên:

- `num` luôn bị ép về `1` — một lần bấm Gán không bao giờ tiêu nhiều IP.
- `country` / `state` / `city` chỉ bị ghi đè khi bạn có chọn ở thanh trên. Để trống thì
  giữ nguyên URL.
- `port` chỉ đổi theo từng cổng khi URL vốn đã có tham số đó (tắt được bằng checkbox).
  Không tự thêm tham số lạ vào URL của hãng khác.

Nút **Thử URL** trong Cài đặt gọi thật rồi cho xem phản hồi nguyên văn cùng những dòng
app đọc được — sai định dạng là thấy ngay, không phải đoán.

Không còn Keychain. Key nằm trong `settings.json` cùng chỗ với mọi cấu hình khác: nó là
cấu hình chứ không phải bí mật, và cất vào Keychain chỉ làm việc sửa / sao lưu / chuyển
máy phiền hơn mà không đổi được gì về an toàn.

---

## Chạy nhiều cổng: cổng nào có hút của cổng nào không?

Có ba đường mà một cổng làm hại cổng khác, và chúng rất khác nhau.

**1. Cạn file descriptor — đây mới là thủ phạm thật, và nó âm thầm.** Giới hạn fd là
của cả tiến trình, không phải từng cổng. Một thiết bị mở vài nghìn kết nối là `accept()`
của MỌI cổng khác bắt đầu trả `EMFILE`; các cổng khác chết mà chẳng liên quan gì tới
băng thông. Tệ hơn: trên macOS, app chạy từ Finder có giới hạn mềm mặc định chỉ **256
fd**, mà mỗi kết nối proxy tốn 2 fd.

Xử lý: app tự nâng giới hạn fd lúc khởi động (`rlimit.rs`), suy ra trần kết nối chung
từ đó, cộng thêm trần riêng cho từng cổng (mặc định 512). Thanh bên hiện số kết nối
đang mở trên tổng trần, và có cảnh báo khi vượt 80%.

**2. Nghẽn đầu hàng.** Không dính: mỗi kết nối là một task tokio riêng trên runtime
nhiều luồng, và tokio tự bắt task nhường lượt sau khoảng 128 thao tác I/O. Test
`cac_cong_truyen_song_song_chu_khong_xep_hang` đo thẳng điều này — 8 cổng chạy cùng lúc
phải nhanh hơn hẳn 8 lần thời gian của một cổng.

**3. Tranh băng thông thật.** Đường mạng của máy chỉ có một. TCP đã chia tương đối đều
giữa các kết nối, và vì mỗi cổng có kết nối riêng nên các cổng tự cạnh tranh sòng phẳng
— **mặc định không đặt trần băng thông**, và khi không đặt thì không tốn một phép tính
nào trên đường chạy nóng. Chỉ đặt trần khi muốn ghìm hẳn một cổng lại, ví dụ proxy tính
tiền theo GB.

---

## App không tự tiêu băng thông của bạn

Nguyên tắc: **không có đường mã nào tự gửi dữ liệu qua proxy.**

| Việc | Đi qua proxy? | Khi nào chạy |
|---|---|---|
| Lấy IP mới (gọi URL nhà cung cấp) | không — gọi thẳng API | khi bấm Gán / Đổi |
| Hiện proxy đã gán trong bảng | không tốn gì | miễn phí, lấy từ chính phản hồi API |
| Ping kiểm tra sống | không — chỉ bắt tay TCP tới gateway, không truyền dữ liệu | khi bấm, hoặc theo chu kỳ nếu bật |
| Xem IP thoát thật (nút «IP?») | **có** | chỉ khi bạn bấm, trên đúng cổng đó |

Bản trước tự dò IP thoát sau mỗi lần gán (đi qua proxy) rồi còn tra vị trí + múi giờ
qua `ip-api.com`. Bỏ hẳn cả hai. Cột trong bảng giờ hiện thẳng `host:port` mà nhà cung
cấp trả về — thông tin đó nằm sẵn trong phản hồi API nên biết nó không tốn thêm byte
nào. Muốn biết IP thoát thật thì bấm «IP?», một lần cỡ vài trăm byte.

---

## Cấu trúc

```
xproxy-2/
├── crates/xproxy-core/         ← lõi, KHÔNG phụ thuộc Tauri
│   ├── src/
│   │   ├── relay/
│   │   │   ├── mod.rs          SlotHandle — vòng đời một cổng (đổi IP không đóng cổng)
│   │   │   ├── socks5.rs       server SOCKS5, phía nghe thiết bị LAN
│   │   │   ├── http.rs         server HTTP CONNECT, phía nghe thiết bị LAN
│   │   │   ├── upstream.rs     client SOCKS5/HTTP, nối proxy nhà cung cấp
│   │   │   ├── pipe.rs         chuyển tiếp 2 chiều + đếm byte
│   │   │   ├── limit.rs        gáo token + trần kết nối (chia tài nguyên giữa cổng)
│   │   │   └── cancel.rs       cờ huỷ dựng trên tokio::sync::watch
│   │   ├── rlimit.rs           nâng giới hạn file descriptor lúc khởi động
│   │   ├── manager.rs          quản lý cả dải cổng  (← ProxyManager.swift)
│   │   ├── provider/           API nhà cung cấp     (← ProxyAPI.swift, Provider.swift)
│   │   ├── net.rs              IP LAN, ping, exit IP (← NetworkUtils/PortUtils.swift)
│   │   ├── addr.rs             địa chỉ đích SOCKS5
│   │   └── error.rs
│   └── tests/
│       ├── relay.rs            10 test cho đường đi cơ bản
│       └── fairness.rs         5 test cho chuyện chia tài nguyên giữa các cổng
├── src-tauri/                  vỏ Tauri
│   └── src/{lib,commands,settings}.rs
└── src/                        React + TypeScript
    ├── lib/ipc.ts              kiểu dữ liệu + wrapper invoke()
    └── views/{Forwarding,Settings}.tsx
```

Lõi tách riêng là có chủ ý: nó build và test được trên máy bất kỳ có Rust, kể cả CI
Linux không có WebView — nên đo tốc độ so với gost mà không cần dựng giao diện.

---

## Đã làm / chưa làm

**Đã có:** relay SOCKS5 + HTTP, đổi IP tại chỗ không đóng cổng, đếm băng thông từng
cổng, trần kết nối + trần băng thông từng cổng, tự nâng giới hạn fd, gán / gỡ / gán
hàng loạt, dán proxy thủ công, nhà cung cấp tuỳ ý qua URL (thêm / sửa / xoá / thử),
ping kiểm tra sống, kiểm tra IP thoát theo yêu cầu, tự xoay IP, tự kiểm tra, CI build
mac + Windows.

**Chưa có:**

- **Đo tốc độ so với gost.** Việc quan trọng nhất còn lại — chưa đo thì chưa biết relay
  này bằng hay hơn gost.
- **Chống-detect / hồ sơ trình duyệt** — bỏ theo thống nhất.
- **Tra vị trí / múi giờ của IP** — bỏ theo thống nhất (không dùng tới, lại tốn một
  lần gọi mạng mỗi lần gán).
- Xác thực user/pass cho cổng LAN: lõi hỗ trợ (`ListenAuth`), chưa nối ra UI.
- Trang quản lý của nhà cung cấp mở bằng trình duyệt hệ thống, không nhúng trong cửa sổ
  như bản Swift.

## Lưu ý khi chạy trên Windows

Lần đầu bind `0.0.0.0` sẽ hiện hộp thoại Windows Defender Firewall — phải bấm Allow,
nếu không thiết bị khác trong LAN không kết nối vào được.
