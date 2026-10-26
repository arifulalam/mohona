# Mohona app

The desktop window (Tauri 2 with React and Tailwind). It is a client of the Mohona engine, like the command line and the browser extension: it finds the engine through `%LOCALAPPDATA%\Mohona\daemon.json`, starts `mohona daemon` when it is not running, and asks for the list of downloads once a second.

```
app/
  src/            the page (React, TypeScript, Tailwind)
  tests/          page tests (Vitest)
  src-tauri/      the thin Rust shell: it only forwards the page's commands
                  to crates/appcore, where the real work and its tests are
```

## Try the page in a browser (no Rust needed)

```
cd app
npm install
npm run dev        # open http://localhost:1420 : a pretend engine with sample downloads
npm run check      # types, tests, build
```

## Run the real app on Windows

You need the Rust toolchain, Node, and Microsoft's WebView2 (already part of Windows 11).

```
cargo build --release -p mohona-cli      # from the repository root; makes target\release\mohona.exe
cd app
npm install
set MOHONA_PROGRAM=..\target\release\mohona.exe
npm run tauri dev                         # development window
npm run tauri build                       # installer in src-tauri\target\release\bundle
```

The app looks for the `mohona` program in `MOHONA_PROGRAM`, then next to the app, then on the `PATH`. (The installer in Phase 8 will put both in the same folder.)
