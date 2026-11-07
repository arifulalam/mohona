# Building and testing Mohona on Windows

One page that takes you from an empty Windows 11 PC to every test that has to be run by hand. Details of each hand test are in `docs/manual-tests.md`; the number in brackets says which section and step.

Work through the parts in order. Stop at the first failure and report it (see "What to send back" at the end).

## Part 0: install the tools (once)

| Tool | How | Check |
|---|---|---|
| Visual Studio Build Tools 2022 (or Visual Studio) with the workload "Desktop development with C++" | https://visualstudio.microsoft.com/visual-cpp-build-tools/ | `where link.exe` in "Developer PowerShell" |
| Rust (MSVC toolchain) | https://rustup.rs, accept the defaults | `rustc --version`, `cargo --version` |
| Node.js 22 LTS | https://nodejs.org | `node --version`, `npm --version` |
| Microsoft Edge WebView2 Runtime | already part of Windows 11 (the app's window needs it) | nothing to do |
| ffmpeg (for joining video and audio of streams) | e.g. `winget install Gyan.FFmpeg`, then open a new terminal | `ffmpeg -version` |
| Chrome or Edge, and Firefox | for the extension tests | |

Use PowerShell. Folder names with spaces are fine.

## Part 1: the engine (Rust) and its automatic tests

From the folder that holds `Cargo.toml` (the `mohona` folder):

```
cargo test --workspace
cargo build --release -p mohona-cli -p mohona-host
```

- The tests take a few minutes the first time. Expected: every line says `test result: ok`.
- If anything fails, copy the failing test's name and the text under it.
- Some tests start local servers on 127.0.0.1; a firewall question can be answered with "Cancel / Block" (nothing leaves the PC).

The build makes `target\release\mohona.exe` and `target\release\mohona-host.exe`.

## Part 2: the browser extension

```
cd extension
npm install
npm run check
npm run package
cd ..
```

- `npm run check` must end with `Tests ... passed` and `built chromium`, `built firefox`.
- `npm run package` writes `extension\dist\store\mohona-chromium-0.0.1.zip` and the Firefox zip.

## Part 3: the app page (no Rust window yet)

```
cd app
npm install
npm run check
```

- Expected: `Tests ... passed` and a finished build.
- Quick look: `npm run dev`, open http://localhost:1420. It shows a pretend engine with sample downloads. Check that it looks right in light and dark (the "Theme" button).

## Part 4: build the desktop window (Tauri)

Still in `app`:

```
$env:MOHONA_PROGRAM = "C:\full\path\to\mohona\target\release\mohona.exe"
npm run tauri dev
```

- Use the **full** path. A relative path does not work, because the window program runs from another folder.
- The first start compiles a lot (5 to 20 minutes, much text). Later starts take seconds.
- Expected: the Mohona window opens with "Mohona is not running" and a "Start Mohona" button.
- To make the installer, run `.\scripts\build-windows.ps1` from the `mohona` folder (PowerShell). It builds the engine and the browser host, copies them next to the app, runs the app's checks and makes the installer in `app\src-tauri\target\release\bundle\nsis\`. The installer contains `mohona.exe` and `mohona-host.exe`; the installed app finds `mohona.exe` next to itself.

If the compile fails, send the **first** `error[...]` block (not the last lines). The Tauri part (`app\src-tauri\src\lib.rs`) has never been compiled before, so errors there are likely and quick to fix.

## Part 5: register the host and load the extension

```
target\release\mohona-host install
```

Then load the extension (`extension\dist\chromium` in Chrome or Edge through `chrome://extensions` > Developer mode > Load unpacked; Firefox: `about:debugging` > This Firefox > Load Temporary Add-on > `extension\dist\firefox\manifest.json`).

## The test list

Tick each line when it passes. "Section N.M" means `docs/manual-tests.md`, section name, step.

### A. Engine and host (about 20 minutes)

- [ ] A1 Daemon starts, a second one says "already running", Ctrl-C removes `%LOCALAPPDATA%\Mohona\daemon.json`, a killed daemon leaves a stale file that is replaced (Browser host and daemon 1 to 2)
- [ ] A2 Host exits quietly with no input (Browser host and daemon 3)
- [ ] A3 `mohona-host install` writes the registry keys and files; `status` lists them; moving the exe shows `PROGRAM NOT FOUND`; `uninstall` removes them, twice is fine; a user folder with Bengali letters or spaces works (Host registration 1 to 4)
- [ ] A4 Starting the engine from the host shows no console window and survives the host (Browser host and daemon 4)
- [ ] A5 Everything unchecked under "Tests still to run by hand" at the top of `docs/manual-tests.md` that you can try (proxies, certificates, FTP, SFTP, WebDAV, mirrors, checksums, power and system behaviour); skip what you have no server for and write "no server"

### B. Browser extension, Chrome or Edge (about 40 minutes)

- [ ] B1 Extension id is `iflnmaneimjgoibmpcmlkllhjccebbhj`; popup says "not running", "Start Mohona" starts it without a console flash (Browser extension 1 to 3)
- [ ] B2 Pasting a link adds it; the same link again is refused (Browser extension 4)
- [ ] B3 Popup says "not connected" before `mohona-host install` (Browser extension 5)
- [ ] B4 Downloads from the browser are taken over; a login-protected download arrives complete; fail-open when Mohona cannot start; private windows and `blob:` stay with the browser (Download interception 1 to 7)
- [ ] B5 Right-click menu on link, image, video; no entry on plain text; error markers when Mohona is down (Context menu 1 to 5)
- [ ] B6 Media list for HLS, DASH and MP4; the list resets per page; protected video offers nothing playable; odd characters in file names (Media detection and streams 1 to 8)
- [ ] B7 Saving `data:` and `blob:` files, large file, duplicate name gets `(1)` (Files only the browser has 1 to 7)
- [ ] B8 Expired link refresh by button and automatically; different file refused (Expired links 1 to 6)
- [ ] B9 Video button on big players, hide per site, never pauses the video, works on strict-CSP sites (On-page video button 1 to 6)
- [ ] B10 Extension settings: the popup's "Which downloads to take over" saves; the same values show in Edge when both are loaded; a change while Mohona is stopped arrives when it starts (Shared settings 1 to 3)
- [ ] B11 Store zips: the Chromium zip loads unpacked with the Mohona icon, has no `key`, and shows a different id; the Firefox zip loads as a temporary add-on (Store packaging 1 to 3)
- [ ] B12 Repeat B1 to B3 in Edge, and in Firefox (Browser extension 6 and 7)
- [ ] B13 Real site with a login (Basic, Digest, cookies) from "Logins and cookies against a real site"

### B2. Extension popup (about 10 minutes)

- [ ] B2 Section "Extension popup" of `docs/manual-tests.md`, steps 1 to 5

### C. Desktop app (about 60 minutes)

Numbers are the steps of the "Desktop app" section.

- [ ] C1 Build and first start: the window opens; "Start Mohona" starts the engine; closing the window leaves the engine running (1, 2)
- [ ] C2 Add, watch progress, pause, resume, cancel; duplicate and bad address messages (3, 4)
- [ ] C3 Remove with and without deleting the file; Ctrl-click for several rows (5, 6)
- [ ] C4 Theme button cycles and is remembered; "system" follows Windows (7)
- [ ] C5 A download started from the browser shows up in the window (8)
- [ ] C6 Settings: speed limit, downloads at once, new folder, relative folder refused (9)
- [ ] C7 Settings, browsers: the site list and options travel to and from the extension (10)
- [ ] C8 Tray icon, close to tray, quit variants (11)
- [ ] C9 Messages for finished and failed downloads; no messages for old ones at start; switch off (12)
- [ ] C10 Open and Show in folder; `.exe` is refused; missing file message (13)
- [ ] C11 Schedule window, outside-hours notice, speed limit in a window, midnight window (14)
- [ ] C12 End action: cancel test, then the real sleep test with your work saved (15)
- [ ] C13 Browse buttons (folder dialog) in Add and Settings (16)
- [ ] C14 Categories add, refused cases, remove (17)
- [ ] C15 Sort by type on and off, also for a browser download (18)
- [ ] C16 Queues: add "Night", duplicate name refused, own downloads-at-once and speed limit, closed window makes its downloads wait while Main runs, queue choice in Add, remove moves downloads to Main (19)
- [ ] C17 Details window: connections, per-connection speed, parts bar, progress never goes backwards (20)
- [ ] C18 Categories in the sidebar and list, Sort by (21)
- [ ] C19 Themes in the app (22)
- [ ] C20 Copied link offer: bar, Download it, Ignore, not offered twice, off by default (23)
- [ ] C21 Languages: Bengali, Arabic (right to left), restart keeps the choice, own language from `docs/translating.md` (24, 25)
- [ ] C22 `.\scripts\build-windows.ps1` makes an installer (see C35 for installing it) (26)
- [ ] C23 Looks: window at 760 px wide, Windows scaling 125% and 150%, light and dark; text readable, nothing cut off, the Save buttons of Settings and Schedule always visible
- [ ] C24 Quiet when idle: with the window hidden for 10 minutes, Task Manager shows the app near 0% CPU
- [ ] C25 Local API: `mohona api`, then the steps of "Local API" in `docs/manual-tests.md` (token works, web page refused, read-only token cannot add)
- [ ] C26 Rules: the steps of "Rules" in `docs/manual-tests.md` (folder routing, block, test, and the app's Rules window)
- [ ] C29 BitTorrent: the steps of "BitTorrent" in `docs/manual-tests.md` (magnet link, resume, firewall prompt, no sharing after finish unless Settings, Torrents says so)
- [ ] C28 Settings window: the steps of "Settings window" in `docs/manual-tests.md` (one button, groups on the left, language and theme in General)
- [ ] C27 Site profiles: the steps of "Site profiles" in `docs/manual-tests.md` (connection cap, speed cap, the app's window)
- [ ] C30 Cloud links: the steps of "Cloud links" in `docs/manual-tests.md` (a Drive, Dropbox, OneDrive and GitHub link each download the real file)
- [ ] C31 Remote companion: the steps of "Remote companion" in `docs/manual-tests.md` (phone on the same Wi-Fi, firewall prompt, read-only token)
- [ ] C32 Settings sync: the steps of "Settings sync" in `docs/manual-tests.md` (two computers or two user accounts, one synced folder)
- [ ] C33 Plugins: the steps of "Plugins" in `docs/manual-tests.md` (the PowerShell example plugin)
- [ ] C34 Torrent file selection: the steps of "Torrent file selection" in `docs/manual-tests.md` (also check that a torrent of several files gets its own folder)
- [ ] C42 Update check: "Update check" in `docs/manual-tests.md`
- [ ] C41 Drop target: "Drop target" in `docs/manual-tests.md` (also check the main window still opens normally and Alt+F4 on it still goes to the tray)
- [ ] C40 Start with Windows: "Start with Windows" in `docs/manual-tests.md`
- [ ] C39 Virus scan: "Virus scan" in `docs/manual-tests.md` (Windows Defender and the EICAR test file)
- [ ] C38 Site Grabber: "Site Grabber" in `docs/manual-tests.md`
- [ ] C37 Torrent sharing and the finish message: "BitTorrent" step 6 and "Finish message" in `docs/manual-tests.md`
- [ ] C36 YouTube and Tools: the steps of "YouTube and Tools" in `docs/manual-tests.md`
- [ ] C35 Installer: the steps of "Installer" in `docs/manual-tests.md` (install on a clean user account, start, browser host registered, uninstall)
- [ ] B4 Extension "Open" and "Show in folder" for finished downloads: the steps of "Extension: open finished downloads" in `docs/manual-tests.md`
- [ ] B3 Extension popup: a very long link in an error message wraps inside its own small scrolling box; the popup page gets no horizontal scrollbar

## What to send back

For each failure:

1. The test number (for example `C8`, or `B4 step 3`).
2. What you did and what happened (one or two sentences; a screenshot helps for the window).
3. For build errors: the **first** `error` block and the command you ran.
4. For engine problems: the output of `target\release\mohona.exe --version` and, if it helps, `%LOCALAPPDATA%\Mohona\daemon.json` without the `token` line (the token is a secret).
5. Browser and version for extension problems.

Passing tests need no report; a short "A and B all passed, C1 to C7 passed" is enough.
