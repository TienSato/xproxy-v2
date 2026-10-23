#!/usr/bin/env bash
# Đóng gói .dmg bằng hdiutil trực tiếp, KHÔNG qua bundle_dmg.sh của Tauri.
#
# bundle_dmg.sh sai bảo Finder qua AppleScript để sắp xếp cửa sổ DMG cho đẹp. macOS chặn
# Terminal điều khiển Finder nếu chưa cấp quyền Automation, thế là cả bước đóng gói chết
# dù app đã build xong. Cách này bỏ phần trang trí, chỉ tạo ảnh đĩa kéo-thả.
#
#   ./scripts-dmg.sh
set -euo pipefail

APP="target/release/bundle/macos/xproxy.app"
VERSION=$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed 's/.*"\([0-9.]*\)".*/\1/')
OUT="target/release/bundle/dmg/xproxy_${VERSION}_aarch64.dmg"
STAGE=$(mktemp -d)

[ -d "$APP" ] || { echo "Chưa có $APP — chạy 'npm run tauri build' trước."; exit 1; }

# Gỡ ảnh đĩa cũ còn dính từ lần build hỏng trước, nếu có.
hdiutil detach "/Volumes/xproxy" >/dev/null 2>&1 || true

# ditto CHỨ KHÔNG PHẢI cp -R.
#
# `cp -R` làm rơi extended attribute, mà chữ ký mã của app nằm ở đó. Trên Apple Silicon
# mọi binary BẮT BUỘC có chữ ký hợp lệ — mất chữ ký là macOS từ chối mở thẳng, chỉ báo
# cụt lủn "Không thể mở ứng dụng". `ditto` giữ nguyên attribute và symlink.
ditto "$APP" "$STAGE/xproxy.app"

# Ký lại ad-hoc cho chắc: nếu bước copy có làm xước chữ ký thì lệnh này vá lại.
codesign --force --deep --sign - "$STAGE/xproxy.app"
codesign --verify --deep --strict "$STAGE/xproxy.app" || {
  echo "!! Chữ ký không hợp lệ — app sẽ không mở được."; exit 1; }

ln -s /Applications "$STAGE/Applications"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"

hdiutil create -volname "xproxy" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
rm -rf "$STAGE"

echo
echo "Xong: $OUT"
