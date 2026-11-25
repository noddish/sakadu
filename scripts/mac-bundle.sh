#!/usr/bin/env bash
# 맥 앱 묶음 Sakadu.app을 만든다 (macOS에서 돈다. CI의 mac-app 작업이 부른다).
# Apple Silicon과 Intel 실행 파일을 하나로 합치고(lipo), Windows 아이콘(.ico)으로 icns를 만들고, Info.plist를 쓰고,
# 애드혹으로 서명한다 (공증 없음: 처음 열 때 '그래도 열기'가 필요하다).
#
# 쓰는 법: scripts/mac-bundle.sh [target 폴더] [출력 폴더]
#   target 폴더: 두 아키텍처를 `cargo build --release --features mac-app --bin sakadu-mac --target …`로 빌드한 곳 (기본 target)
#   출력 폴더: Sakadu.app을 둘 곳 (기본 dist). 버전은 저장소의 Cargo.toml에서 읽는다
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET=${1:-target}
OUT=${2:-dist}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
APP="$OUT/Sakadu.app"

# 늘 새로 묶는다 (지난번 묶음에 남은 파일이 섞이지 않게)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/sakadu-mac" \
  "$TARGET/aarch64-apple-darwin/release/sakadu-mac" "$TARGET/x86_64-apple-darwin/release/sakadu-mac"
lipo -archs "$APP/Contents/MacOS/sakadu-mac"

# 아이콘: Windows 아이콘(.ico)을 PNG로 바꿔 icns로 (안 되면 아이콘 없이 묶는다)
if sips -s format png "$ROOT/assets/sakadu.ico" --out "$OUT/icon.png" >/dev/null; then
  mkdir -p "$OUT/AppIcon.iconset"
  for s in 16 32 128 256; do
    sips -z $s $s "$OUT/icon.png" --out "$OUT/AppIcon.iconset/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2)); sips -z $d $d "$OUT/icon.png" --out "$OUT/AppIcon.iconset/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$OUT/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns" || echo "icns 실패: 아이콘 없이 묶음"
fi

# Info.plist.
# - 폴더를 여는 앱으로 밝힌다 (CFBundleDocumentTypes: public.folder, 보기 전용, 기본 앱이 아닌 '다른 앱'의 자리).
#   Finder의 '다음으로 열기'와 Dock 아이콘에 폴더를 놓을 때 Sakadu가 받는다 (src/mac/view.rs의 application:openURLs:)
# - 서비스 메뉴의 'Sakadu에서 열기' (NSServices): Finder에서 고른 폴더·파일을 보낸다. 받는 쪽은 앱 위임의
#   openInSakadu:userData:error:다. NSRequiredContext를 두어야 사용자가 따로 켜지 않아도 메뉴에 보인다
# - LSUIElement는 두지 않는다: Dock 아이콘과 메뉴가 있는 보통 앱이다. 창을 닫고 기다리는 것은 메뉴 막대 아이콘이 한다
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Sakadu</string>
  <key>CFBundleDisplayName</key><string>Sakadu</string>
  <key>CFBundleIdentifier</key><string>io.github.noddish.sakadu</string>
  <key>CFBundleExecutable</key><string>sakadu-mac</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>GPL-3.0-or-later</string>
  <key>NSDesktopFolderUsageDescription</key><string>사카두에서 데스크탑의 파일을 보고 이름으로 찾을 수 있게 해요.</string>
  <key>NSDocumentsFolderUsageDescription</key><string>사카두에서 문서 폴더의 파일을 보고 이름으로 찾을 수 있게 해요.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>사카두에서 다운로드 폴더의 파일을 보고 이름으로 찾을 수 있게 해요.</string>
  <key>NSFileProviderDomainUsageDescription</key><string>사카두에서 iCloud Drive의 파일을 보고 이름으로 찾을 수 있게 해요.</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Folder</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.folder</string>
      </array>
    </dict>
  </array>
  <key>NSServices</key>
  <array>
    <dict>
      <key>NSMenuItem</key>
      <dict>
        <key>default</key><string>Sakadu에서 열기</string>
      </dict>
      <key>NSMessage</key><string>openInSakadu</string>
      <key>NSPortName</key><string>Sakadu</string>
      <key>NSRequiredContext</key><dict/>
      <key>NSSendFileTypes</key>
      <array>
        <string>public.folder</string>
        <string>public.item</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
EOF
plutil -lint "$APP/Contents/Info.plist"
codesign --force --deep --sign - "$APP"
codesign --verify --verbose "$APP"
echo "묶음: $APP ($VERSION)"
