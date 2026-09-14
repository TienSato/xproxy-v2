# Đóng gói xproxy

Hai bản: macOS Apple Silicon (`.dmg`) và Windows x64 (`.exe` / `.msi`).

> **Tauri không cross-compile được.** Từ máy Mac không tạo ra được file cài Windows,
> và ngược lại. Mỗi hệ điều hành phải build trên chính nó — hoặc nhờ GitHub Actions
> build hộ (§3).

---

## 1. macOS — file `.dmg`

### Chuẩn bị (chỉ làm một lần)

```bash
cd /Users/sato/Documents/xproxy-2
npm install
```

Icon đã có sẵn trong `src-tauri/icons/`, không cần sinh lại.

### Build

```bash
npm run tauri build
```

Ra file:

```
target/release/bundle/dmg/xproxy_0.1.0_aarch64.dmg
```

Bản này chạy trên máy **Apple Silicon** (M1/M2/M3/M4). Máy Mac chip Intel không mở được
— đã thống nhất không hỗ trợ.

### Máy nhận phải làm gì

App chưa ký bằng tài khoản Apple Developer, nên macOS sẽ chặn ở lần mở đầu tiên với
thông báo kiểu *"không mở được vì không xác minh được nhà phát triển"*. Bảo người nhận:

1. Mở `.dmg`, kéo **xproxy** vào Applications.
2. Vào Applications, **chuột phải** lên xproxy → **Open** → bấm **Open** lần nữa.

Chỉ cần làm một lần. Từ lần sau mở bình thường.

Nếu vẫn bị chặn, chạy lệnh này trên máy đó rồi mở lại:

```bash
xattr -cr /Applications/xproxy.app
```

Lần chạy đầu macOS cũng sẽ hỏi cho phép nhận kết nối mạng đến — phải bấm **Allow**,
không thì thiết bị khác trong LAN không cắm vào được.

---

## 2. Windows — file `.exe` và `.msi`

Phải chạy trên **một máy Windows thật** (hoặc máy ảo Windows, hoặc Parallels/UTM trên
Mac). Chép cả thư mục dự án sang, trừ `node_modules/` và `target/`.

### Chuẩn bị trên máy Windows (chỉ làm một lần)

1. **Visual Studio Build Tools** — tải từ trang Microsoft, khi cài chọn workload
   **"Desktop development with C++"**. Không có nó thì Rust không link được.
2. **Rust**: tải `rustup-init.exe` từ <https://rustup.rs>, chạy, chọn mặc định.
3. **Node.js 22**: tải từ <https://nodejs.org>.
4. **WebView2** — Windows 11 có sẵn. Windows 10 thì bộ cài của app tự tải về, không
   cần làm gì.

Khởi động lại PowerShell sau khi cài Rust.

### Build

```powershell
cd C:\duong\dan\toi\xproxy-2
npm install
npm run tauri build
```

Ra hai file, gửi cái nào cũng được:

```
target\release\bundle\nsis\xproxy_0.1.0_x64-setup.exe    ← bộ cài, quen thuộc hơn
target\release\bundle\msi\xproxy_0.1.0_x64_en-US.msi     ← gói MSI, hợp môi trường công ty
```

### Máy nhận phải làm gì

Chưa ký số nên SmartScreen sẽ cảnh báo *"Windows protected your PC"*. Bấm
**More info** → **Run anyway**.

Lần chạy đầu Windows Defender Firewall sẽ hỏi cho phép — **phải bấm Allow**, nếu không
thiết bị khác trong LAN không kết nối vào được.

---

## 3. Không có máy Windows? Để GitHub build hộ

Cách này ra **cả hai** bản cùng lúc, miễn phí, không cần sở hữu máy Windows.

### Lần đầu

```bash
cd /Users/sato/Documents/xproxy-2
git init
git add .
git commit -m "xproxy v2"
```

Tạo một repo **private** trên GitHub, rồi:

```bash
git remote add origin https://github.com/<tên-bạn>/xproxy-2.git
git branch -M main
git push -u origin main
```

### Mỗi lần muốn ra bản mới

```bash
git add .
git commit -m "bản mới"
git tag v0.1.0
git push && git push --tags
```

Đẩy tag lên là GitHub tự build cả macOS lẫn Windows rồi tạo một **Release nháp** kèm
`.dmg` và `.exe`. Vào tab **Releases** của repo, kiểm tra rồi bấm Publish. Mất khoảng
10–15 phút.

Không muốn gắn tag thì cứ push bình thường: vào tab **Actions**, mở lần chạy mới nhất,
kéo xuống mục **Artifacts** để tải file về.

Cấu hình nằm ở `.github/workflows/build.yml`, không phải sửa gì.

---

## 4. Dọn chỗ

Thư mục `target/` là bộ nhớ đệm biên dịch, chiếm khoảng **2 GB**. Xoá được, chỉ là lần
build sau sẽ lâu hơn:

```bash
cargo clean
```

`node_modules/` (80 MB) lấy lại bằng `npm install`.

Cả hai đều đã nằm trong `.gitignore`, không bị đẩy lên GitHub.

---

## 5. Đổi số phiên bản

Sửa ở **hai** chỗ cho khớp nhau:

- `package.json` → `"version"`
- `src-tauri/tauri.conf.json` → `"version"`

Số này đi vào tên file cài, nên đổi trước khi build bản gửi đi.
