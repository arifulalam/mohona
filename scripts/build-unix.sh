#!/usr/bin/env bash
# Builds the Mohona app for Linux (.deb, .AppImage) or macOS (.app, .dmg).
# Run it on that system, from anywhere:   ./scripts/build-unix.sh
#
# Linux needs the system packages listed in docs/BUILDING.md first. The engine (`mohona`) and the
# browser host (`mohona-host`) are built in release mode and bundled as sidecars, so they sit next
# to the app program, which is where the app looks for them; the browser extension goes into the
# bundle's `extension` folder.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

case "$(uname -s)" in
  Linux)  config="src-tauri/tauri.bundle.linux.conf.json" ;;
  Darwin) config="src-tauri/tauri.bundle.macos.conf.json" ;;
  *) echo "This script is for Linux and macOS; on Windows use scripts/build-windows.ps1." >&2; exit 1 ;;
esac
triple="$(rustc -vV | sed -n 's/^host: //p')"

echo "1/4 Engine and browser host (release)..."
cargo build --release -p mohona -p mohona-host

echo "2/4 Sidecars and the browser extension..."
mkdir -p app/src-tauri/binaries
cp target/release/mohona "app/src-tauri/binaries/mohona-$triple"
cp target/release/mohona-host "app/src-tauri/binaries/mohona-host-$triple"
(cd extension && npm ci && npm run build)
rm -rf app/src-tauri/resources/extension
mkdir -p app/src-tauri/resources/extension
cp -R extension/dist/chromium app/src-tauri/resources/extension/chromium
cp -R extension/dist/firefox app/src-tauri/resources/extension/firefox

echo "3/4 App dependencies and checks..."
cd app
npm ci
npm run check

echo "4/4 The app and its packages (the first time this takes a long while)..."
npm run tauri -- build --config "$config"

echo
echo "Done. The packages:"
find src-tauri/target/release/bundle -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.dmg' \) -print
