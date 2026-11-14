# Building Mohona

## What you need

- [Rust](https://rustup.rs) (stable) and [Node.js](https://nodejs.org) 22.
- **Windows:** the Visual Studio C++ Build Tools and WebView2 (already part of Windows 11).
- **Linux:** `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev patchelf build-essential`
- **macOS:** Xcode command-line tools (`xcode-select --install`).

## The engine and the command line

```
cargo test --workspace
cargo run -p mohona -- --version
```

## The desktop app and its installer

| System | Command | Result (under `app/src-tauri/target/release/bundle/`) |
| --- | --- | --- |
| Windows | `.\scripts\build-windows.ps1` | `nsis\Mohona_<version>_x64-setup.exe` |
| Linux | `./scripts/build-unix.sh` | `deb/…deb`, `appimage/…AppImage` |
| macOS | `./scripts/build-unix.sh` | `dmg/…dmg`, `macos/Mohona.app` |

The scripts build the engine and the browser host, the extension, check the app, and build the installer.

## The browser extension

```
cd extension
npm ci
npm test
npm run build      # dist/chromium and dist/firefox, to load unpacked
npm run package    # dist/store/*.zip, for the stores (see store.md)
```

## All platforms from GitHub

`.github/workflows/release.yml` builds Windows, Linux, macOS (Apple silicon and Intel) and the extension zips, with a `.sha256` file for each, when you push a tag:

```
git tag v0.1.0
git push --tags
```

The result is a **draft** release; read it, then press *Publish*. The landing page reads the latest published release, and the app's "Check for updates" does the same.

`.github/workflows/ci.yml` runs the tests on Windows, Linux and macOS for every push. `.github/workflows/pages.yml` publishes `site/` to GitHub Pages: in the repository, Settings → Pages → Source: **GitHub Actions** (once).

## iOS and Android

Not built, on purpose. The app is a desktop program: it starts the engine as a separate program and talks to browsers through a native-messaging host, and neither exists on iOS or Android (apps cannot start such programs; phones have no desktop browser extensions). A phone version would be a separate project: the engine as a library, a touch interface and the platform's download services. What works today is the **web companion** (`mohona remote`): run Mohona on a computer or server and open its page from a phone on the same network.
