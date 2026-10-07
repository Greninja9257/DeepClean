#!/bin/bash
# Builds build/DeepClean.app: the Rust engine + the SwiftUI front end.
#   ./build-app.sh            build
#   ./build-app.sh --install  build and copy to /Applications
#   ./build-app.sh --dmg      build and package build/DeepClean-<version>.dmg
set -euo pipefail
cd "$(dirname "$0")"

APP=build/DeepClean.app
SWIFTC="/usr/bin/xcrun swiftc"   # bypass any swiftly shim on PATH

# Some Command Line Tools installs ship a stale duplicate of the SwiftBridging
# module map, which breaks every Foundation import. Mask it with an empty file
# via a VFS overlay (no sudo / system changes needed).
mkdir -p build
STALE=/Library/Developer/CommandLineTools/usr/include/swift/module.modulemap
if [ -f "$STALE" ] && [ -f "$(dirname "$STALE")/bridging.modulemap" ]; then
  : > build/empty.modulemap
  cat > build/vfs-overlay.yaml <<YAML
{ "version": 0, "roots": [ { "type": "file", "name": "$STALE",
  "external-contents": "$PWD/build/empty.modulemap" } ] }
YAML
  SWIFTC="$SWIFTC -vfsoverlay $PWD/build/vfs-overlay.yaml -Xcc -ivfsoverlay -Xcc $PWD/build/vfs-overlay.yaml"
fi

echo "▸ Building engine (Rust)…"
cargo build --release --quiet

echo "▸ Building app (SwiftUI)…"
mkdir -p build
$SWIFTC -O -swift-version 5 -target arm64-apple-macos14.0 -parse-as-library \
  app/*.swift -o build/DeepClean

if [ ! -f build/AppIcon.icns ] || [ scripts/make-icon.swift -nt build/AppIcon.icns ]; then
  echo "▸ Rendering icon…"
  $SWIFTC -O scripts/make-icon.swift -o build/make-icon
  rm -rf build/AppIcon.iconset
  build/make-icon build/AppIcon.iconset
  iconutil -c icns build/AppIcon.iconset -o build/AppIcon.icns
fi

echo "▸ Assembling bundle…"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp build/DeepClean "$APP/Contents/MacOS/DeepClean"
cp target/release/deepclean "$APP/Contents/MacOS/deepclean-engine"
cp build/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DeepClean</string>
  <key>CFBundleDisplayName</key><string>DeepClean</string>
  <key>CFBundleIdentifier</key><string>dev.deepclean.DeepClean</string>
  <key>CFBundleExecutable</key><string>DeepClean</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSAppleEventsUsageDescription</key><string>DeepClean asks macOS for your password to clean system-wide caches.</string>
</dict>
</plist>
PLIST

echo "▸ Signing (ad-hoc)…"
codesign --force --sign - "$APP/Contents/MacOS/deepclean-engine"
codesign --force --sign - "$APP"

if [ "${1:-}" = "--dmg" ]; then
  echo "▸ Creating disk image…"
  DMG="build/DeepClean-${VERSION}.dmg"
  STAGE=build/dmg
  rm -rf "$STAGE" "$DMG"
  mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -quiet -volname "DeepClean" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
  rm -rf "$STAGE"
  echo "✔ Built $DMG"
elif [ "${1:-}" = "--install" ]; then
  rm -rf /Applications/DeepClean.app
  cp -R "$APP" /Applications/
  echo "✔ Installed to /Applications/DeepClean.app"
else
  echo "✔ Built $APP"
fi
