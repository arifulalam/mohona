# Tests still to run by hand

The automatic tests (`cargo test --workspace`) cover all of these on a test server or test proxy. This list is for things that can only be proven against the real world and have **not been run on Windows yet**. Tick a box and write the date when done.

Format: **what to run** → **what should happen**.

## Proxies (no proxy was available)

- [ ] HTTP proxy: `mohona get URL --proxy http://HOST:PORT -o downloads` → file downloads; the proxy's log shows the request.
- [ ] HTTP proxy with a login: `--proxy http://user:pass@HOST:PORT` → downloads. Without `user:pass@` → error telling you the proxy asked for a login.
- [ ] HTTPS site through an HTTP proxy (it uses CONNECT): `get https://... --proxy http://HOST:PORT` → downloads.
- [ ] SOCKS5: `--proxy socks5://HOST:1080` (for example `ssh -D 1080 somehost`, or Tor at `127.0.0.1:9050`) → downloads.
- [ ] SOCKS5h: `--proxy socks5h://HOST:1080` with a name only the proxy can resolve → downloads.
- [ ] System proxy: set a proxy in Windows settings, run `get URL` with no flag → goes through it. Run with `--no-proxy` → goes direct.
- [ ] Saved proxy: `queue add URL --proxy ...`, close the terminal, `queue run` → still uses the proxy.

## Certificates (TLS)

- [ ] A normal HTTPS site (for example `https://www.rust-lang.org/`) downloads with no flags: the Windows trusted authorities are used.
- [ ] A site with a wrong or expired certificate (`https://expired.badssl.com/`, `https://wrong.host.badssl.com/`, `https://self-signed.badssl.com/`): refused with a clear reason; with `--insecure` it downloads.
- [ ] Your own authority: export it as PEM, run `get URL --ca-file ca.pem` → downloads without `--insecure`.
- [ ] A company or antivirus that inspects HTTPS: if normal sites fail, `--ca-file` with its authority fixes it.

## Checksums

- [ ] A real download that publishes a checksum (a Linux ISO with a `SHA256SUMS`-style file, or a `.sha256` next to the file): `get URL --checksum sha256:<published>` → `Checked: sha256 matches`.
- [ ] Same with `--find-checksum` where `URL.sha256` exists → matches and names the checksum file. (A shared `SHA256SUMS` file at another address is not looked for; use `--checksum`.)
- [ ] Flip one character of the checksum → fails, file is kept in the folder.
- [ ] A large file (1 GB or more): the check takes a moment after the download and the numbers still match what `certutil -hashfile FILE SHA256` prints in Windows.
- [ ] A site that sends a `Digest` or `Content-MD5` header (try a few downloads with `-v`): checked automatically with no flag.

## Mirrors

- [ ] A real file with a main address and one or more mirrors (a Linux ISO often has several): `get MAIN --mirror OTHER -v --checksum sha256:...` → downloads, the checksum matches, and the verbose output shows more than one address used.
- [ ] Turn one address off midway (disconnect it, or use a wrong second address) → the download still finishes.
- [ ] A mirror with an older version of the file (same name, different size) → skipped with a note, not mixed in.
- [ ] Mirrors with the same size but different content cannot be told apart by size: confirm `--checksum` catches it.

## FTP

- [ ] A public FTP server (for example `ftp://ftp.gnu.org/gnu/hello/hello-2.12.1.tar.gz`, or any distribution mirror): `get URL -o downloads` → downloads with several connections; compare its size with the site's.
- [ ] Windows firewall: the first run may ask to allow Mohona; passive FTP only makes outgoing connections, so it should work even if you say no.
- [ ] A server needing a login (a router's USB share, a home server, FileZilla Server): `get ftp://host/file -u user` → downloads; wrong password → clear 530 message.
- [ ] Stop with Ctrl+C midway and run again → continues from where it stopped.
- [ ] A server that limits connections per address (many free ones allow 2 to 5): `get URL -c 8` → still finishes, using fewer connections.
- [ ] A server with files that have spaces or non-English letters in the name.

## FTPS

- [ ] A server with explicit encryption (FileZilla Server, vsftpd with `ssl_enable=YES`, many hosting providers): `get ftpes://host/file -u user` → downloads. With a self-made certificate, expect the "not signed by an authority" error first, then success with `--ca-file` or `--insecure`.
- [ ] A server with implicit encryption (port 990): `get ftps://host/file -u user`.
- [ ] **vsftpd defaults to requiring TLS session reuse on data connections.** Mohona reuses sessions; if it fails with `522` or `425` on such a server while a normal client works, report it with the server's name and version.
- [ ] Download with several connections (`-c 4`) from an encrypted server, then compare the file with its checksum.
- [ ] Wireshark or the server log: nothing readable (login, file names) crosses the network before the encryption starts.

## SFTP

- [ ] A real SSH server (a Linux box, a Raspberry Pi, a NAS, a hosting account): `get sftp://user@host/path/file -u user:password --trust-new-host` → downloads, and `.ssh\known_hosts` in your Windows user folder gets a new line.
- [ ] Run it again without `--trust-new-host` → still works (the server is known now).
- [ ] Compare the fingerprint in the first error (run without `--trust-new-host` against a new server) with `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on the server → same `SHA256:` value.
- [ ] Key login: `--key C:\Users\you\.ssh\id_ed25519` (no passphrase) → works. With a passphrase: needs `MOHONA_KEY_PASSPHRASE` set, otherwise the error mentions it. Try an RSA key (older servers) too.
- [ ] With keys in the default place (`.ssh\id_ed25519`) and no `-u` password: `get sftp://user@host/file` works.
- [ ] A server that allows few connections (`MaxSessions 2`): `-c 8` still finishes.
- [ ] Reinstall or switch the server's host key (or edit known_hosts to a different key) → clear "not the one recorded" warning and no login attempt.
- [ ] Servers with only old algorithms (very old routers): if `SSH problem:` appears while OpenSSH works, report the server's name and version.
- [ ] Large file (1 GB+) with a checksum, over a real link.

## WebDAV

- [ ] A Nextcloud or ownCloud account (or a NAS with WebDAV): `get webdavs://host/remote.php/dav/files/USER/file.pdf -u USER:app-password` → downloads; wrong password → clear 401 message.
- [ ] A WebDAV share with Digest login (Apache `AuthType Digest`, some NAS boxes) → downloads.
- [ ] A big file with `-c 4`: check it comes in pieces and the checksum matches (some WebDAV servers do not support ranges: then it must still download as one piece).
- [ ] `webdav://` (plain, no encryption) on a home network server.

## Streams (HLS and DASH)

Automated tests use a local test server. These need real streams from the internet, so run them on your own PC. Use only streams you are allowed to download.

- [ ] **Public HLS test stream.** Pick a public sample stream (for example the Apple or Bitmovin sample streams) and run `mohona stream <master.m3u8> --list`. Expected: the qualities, sound-only variants and audio languages are listed and look right.
- [ ] **Download a short HLS video.** Run `mohona stream <url> -q 360 -o test`. Expected: `test\NAME.ts` (or `.mp4`), plays in VLC with sound (if the audio is separate, `NAME.audio.*` plays too).
- [ ] **Stop and resume.** Start a longer stream, press Ctrl-C halfway, run the same command. Expected: it says how many segments were kept (progress continues, not from 0) and the final file plays from start to end.
- [ ] **AES-128 stream.** Download a stream whose playlist has `#EXT-X-KEY:METHOD=AES-128` (many public samples). Expected: the video plays; no garbled picture or sound.
- [ ] **Joining with ffmpeg.** Install ffmpeg (`winget install ffmpeg`, then open a new terminal), download a stream with separate audio (most DASH ones, or HLS with an `AUDIO` group). Expected: `Joining video and audio with ffmpeg ...`, one `NAME.mp4` with picture and sound, and the two partial files gone. Repeat with `--container mkv`.
- [ ] **No ffmpeg.** Without ffmpeg on the PATH: the same download ends with "saved as two files because ffmpeg was not found" and both files play on their own. Then `--ffmpeg C:\\path\\to\\ffmpeg.exe` (or the folder) works.
- [ ] **A join that fails.** If a stream's codecs do not fit MP4, the message starts with "ffmpeg could not join" and both files are kept; `--container mkv` then works.
- [ ] **Subtitles.** On a stream that has subtitles, run `mohona stream <url> --list`, then `--subs en` (or a language it lists). Expected: `NAME.en.srt` opens in a text editor with numbered cues, and loads in VLC next to the video with correct timing. Try a stream whose subtitles are split over many files, and check there are no repeated lines at the cuts.
- [ ] **Embedded subtitles.** With ffmpeg installed: `--subs en,bn --embed-subs` (use languages the stream lists). Expected: the one `.mp4` shows a subtitle menu in VLC with both languages named; `--container mkv` works too.
- [ ] **Subtitle timing on HLS.** Some HLS streams start their video at a time other than 0 (the `X-TIMESTAMP-MAP` line in the `.vtt`). Check the `.srt` lines up with the speech; if it is off by a constant amount, report the stream: Mohona does not apply that offset yet.
- [ ] **Live HLS.** Find a public live HLS stream (a 24/7 news or test channel) and run `mohona stream <url> --record-for 2m`. Expected: it says it is recording, shows growing time, stops by itself near 2 minutes, and the `.ts` plays in VLC.
- [ ] **Stop a live recording.** Same, without `--record-for`; press Ctrl-C after a minute. Expected: "Saved ... stopped" and a playable file.
- [ ] **Live on a slow link.** Record a high-quality live stream while the network is busy. Expected: a "segment(s) were gone" warning is possible, never a crash, and the file still plays.
- [ ] **Live with separate audio.** A live stream whose audio is a separate playlist: with ffmpeg installed expect one joined `.mp4` after stopping.
- [ ] **Public DASH stream.** Run `mohona stream <manifest.mpd> --list` and then download the lowest quality. Expected: video and audio files that play.
- [ ] **Stream behind a login or cookie.** If you have one, try `--cookie` or `-H`. Expected: works like `get` does; a wrong or missing cookie gives the "key is N bytes, not 16" or 403 message, not a crash.
- [ ] **DRM stream.** Try any Widevine or FairPlay protected sample. Expected: "this stream is protected ... not supported" before anything is downloaded.
- [ ] **Speed limit.** `mohona stream <url> -l 200K`. Expected: total speed stays near 200 KiB/s.
- [ ] **Many segments.** A 1 to 2 hour stream (thousands of segments) with `-c 16`. Expected: it finishes, memory stays low, and the `.parts` folder is gone afterwards. Note the free disk space needed: the pieces plus the final file.

## Preview and convert

- [ ] **Preview an unfinished download.** Start `mohona get <big video URL> -o test`, press Ctrl-C after a while, then `mohona preview <same URL> -o test`. Expected: your default player (on Windows) opens the video and plays until the end of what was downloaded; the download still resumes and finishes afterwards.
- [ ] **Preview with several connections.** Same with `-c 8` (the file is filled in pieces): the preview must still play from the start (it only uses the unbroken start).
- [ ] **Convert to MP3 / M4A.** `mohona convert video.mp4 -p mp3` and `-p m4a`. Expected: files that play in any music player, with the right length.
- [ ] **Convert to a small MP4.** `mohona convert video.mkv -p mp4-small`. Expected: noticeably smaller file, not taller than 720 lines, plays with sound everywhere (also on a phone).
- [ ] **Windows paths.** Convert a file in a folder whose name has spaces and non-English letters (for example a Bangla folder name). Expected: it works and the result is next to the file.

## Browser host and daemon

1. Build: `cargo build -p mohona-cli -p mohona-host`. In one terminal: `target\debug\mohona --db test.db daemon` (expect "Mohona is running"). A second `mohona daemon` must say "already running".
2. Press Ctrl-C in the first terminal; `%LOCALAPPDATA%\Mohona\daemon.json` must disappear. Kill the process from Task Manager instead; the file stays, and the next `mohona daemon` must still start (stale file replaced).
3. Start the daemon again, then `target\debug\mohona-host --db test.db --engine target\debug\mohona.exe` with no input and stop it with Ctrl-Z, Enter; it must exit quietly (exit code 0, nothing on stdout).
4. With the daemon stopped, the host's `startEngine` message must start `mohona daemon` without a console window flashing, and it must keep running after the host exits. (Needs a real browser or a small script to send a message; the real browser check comes with host registration in the next step.)

## Host registration (Windows)

1. `target\debug\mohona-host install --extension-id abcdefghijklmnopabcdefghijklmnop`. Then `reg query HKCU\Software\Google\Chrome\NativeMessagingHosts\org.mohona.host` must show the manifest path, and `reg query HKCU\Software\Mozilla\NativeMessagingHosts\org.mohona.host` the Firefox one. Open the JSON files: `path` must be the full path of `mohona-host.exe`, and the origin must match.
2. `mohona-host status` lists the browsers; move the exe and `status` must say `PROGRAM NOT FOUND`.
3. `mohona-host uninstall`; the keys and files must be gone. Run it twice: the second time says `was not registered`.
4. Paths with spaces or non-English letters (for example a user folder with Bengali letters) must work in step 1 and in the browser later.

## Browser extension

1. `cd extension`, `npm install`, `npm run build`. In Chrome or Edge open `chrome://extensions`, switch on Developer mode, "Load unpacked", pick `extension\dist\chromium`. The extension's id must be `iflnmaneimjgoibmpcmlkllhjccebbhj`.
2. `target\debug\mohona-host install` (registers that id for Chrome, Chromium and Edge, and Firefox's id for Firefox).
3. Click the Mohona toolbar button. With nothing running it must say "Mohona is not running." and show "Start Mohona". Click it: the text must change to "Mohona 0.0.1 is running." and no console window may flash.
4. Paste a real download link and press the button: "Added to Mohona." and the file must appear in your Downloads folder. Press it again: "Not added: already in the list ...".
5. Before `mohona-host install` (or after `uninstall`) the popup must say it is not connected and tell you to run `mohona-host install`.
6. Firefox: `about:debugging` > This Firefox > Load Temporary Add-on > `extension\dist\firefox\manifest.json`; repeat 3 to 5.
7. Edge: repeat 1 to 4 (Edge reads the same Chromium build).

## Download interception

1. With the extension loaded, the host registered and "Take over browser downloads" ticked, click a normal download link (a zip or exe). The browser must not show its own download, and the file must appear in your Downloads folder through Mohona. The popup shows "Sent to Mohona: ...".
2. Log in to a site that needs a login for downloads (a forum attachment, Google Drive, your own server) and download from there: the file must arrive complete, not as a login page.
3. Fail open: close Mohona (stop `mohona daemon`) and rename `mohona.exe` so that it cannot start. Click a download link: the browser must download it normally, and the popup says "Left to the browser (...)".
4. Untick "Take over browser downloads": the browser handles downloads again. A download in an Incognito or private window is never taken.
5. Links that must stay in the browser: a `blob:` download (save an image from a web app), "Save as" of a data URL.
6. Click the same link twice: the second time the browser downloads it itself and the popup says Mohona already has it.
7. The store version will list "read and change all your data on all websites" because of the cookie access; check that you are comfortable with that wording before store submission.

## Context menu

1. Right-click a link to a file, then "Download with Mohona": the file arrives in Downloads through Mohona; the toolbar button shows a green check for a few seconds; the popup says "Sent to Mohona: ...".
2. Right-click an image and a video (or audio) player: the entry is there, and the media file is downloaded. A video from a streaming site that uses `blob:` does nothing there; use the toolbar popup's media list instead.
3. Right-click ordinary text or the page background: no Mohona entry.
4. Stop Mohona and make it unable to start: the entry shows a red "!" and the popup says "Not sent: ...".
5. Use the entry on a link already in Mohona's list: red "!" and "Mohona already has it".

## Media detection and streams

1. Open a page that plays a video from an HLS stream (many news and lecture sites; a public test stream such as the Apple or Bitmovin sample pages). Start the video. The toolbar button must show a number, and the popup lists "HLS" entries under "Media on this page".
2. Press the download button of one entry. The popup says "Stream download started. It will be saved in ...". After a short while the file appears in that folder (`.ts`, or `.mp4` when ffmpeg is installed). It must play.
3. A page that plays a plain MP4 from a normal address: it is listed as "file", and the button adds it to Mohona's list and downloads it with several connections.
4. Open a new page in the same tab: the list and the number must reset. Close the tab: nothing is left behind.
5. A site with protected video (Netflix, Prime Video): nothing downloadable may be offered, or a download must fail with a clear "protected" message and no broken file. Mohona does not remove DRM.
6. Streams that need a login: log in to the site first, then download; the stream must come with your session (cookies and referrer).
7. Stop Mohona and make it unable to start: pressing the button shows an error; nothing breaks in the page.
8. Page titles with odd characters (quotes, slashes, Bengali): the saved file name must be valid on Windows.

## Files only the browser has (blob: and data:)

1. On a page with an image that is a `data:` image (a web app preview, a screenshot tool), right-click it, "Download with Mohona". A file appears in Downloads with the right extension (`.png`, not no extension), and the toolbar shows a check.
2. A web app that creates a file in the page (an online editor's "export" link, an image editor's preview, a PDF viewer built into a site): right-click the link or image, "Download with Mohona". The file arrives complete; compare its size and open it.
3. A file over 10 MB (several pieces): it arrives with the same size and opens. Do the same with a file near 500 MB if you can; memory use of the browser must not jump by that size.
4. Save the same thing twice: the second file is `name (1).ext`, never a replacement.
5. Right-click a `blob:` video from a streaming site: it saves the small piece the page holds or fails with a clear message; it is not a way around protected video.
6. Stop Mohona and make it unable to start: the red "!" appears and the popup says "Not saved: ...".
7. A page title with odd characters (`:`, `?`, quotes, a very long title, only dots): the file name is valid on Windows.

## Expired links

1. Find a site whose download links expire (a file host or a course site that signs its links; or leave a paused download in Mohona for a few hours). Start a download from it and pause or stop Mohona's network until the link expires, then resume. It fails with "the server answered 403" (or 410).
2. Open the extension popup: the download is listed under "Expired links". Press "Refresh link": a tab opens in the background for a moment and closes; the download continues from where it stopped (the progress does not restart from zero) and finishes. Compare the file with a fresh download of it.
3. Leave "Refresh expired links automatically" ticked and wait a minute or two: the same happens without pressing anything. Untick it: nothing happens by itself.
4. A site where the new link is a different file (a new version): Mohona refuses with a message about the size, the version or the name and keeps the download as it was; the file is never mixed up.
5. A page that needs a click or a login you do not have in the browser: the popup says no matching link was found; the download stays in the list.
6. While the refresh runs the background tab must not steal focus or play sound.

## Logins and cookies against a real site

- [ ] A real HTTP Basic site: `get URL -u user:password` → downloads; wrong password → `refused the user name or password`.
- [ ] A real Digest site → downloads.
- [ ] https://www.trainedbyjp.com/downloads/4k_Thetestdata.mp4 after the server is fixed: `--cookie` with a real session cookie → downloads (earlier answer was 401, then 500 from the server).

## On-page video button

1. Open a page with a large video (for example a news site) and look at the video's top right corner: a dark "⬇ Mohona" button appears, faint until the mouse is over it. Small players (ads, previews) get none.
2. Press play, wait a few seconds, click the button. The panel lists the video (file, HLS or DASH). "Download" sends it; the button says "Sent ✓" and the download appears in Mohona.
3. Scroll the page and resize the window: the button follows the video and disappears when the video is mostly out of view. Full screen: the button is not shown (the player covers it); leave full screen and it returns.
4. Click "Hide on <site>": the button goes away at once on every tab of that site and stays away after a restart. Untick "Show a download button on videos" in the popup: it goes away everywhere.
5. Clicking the button must never pause or start the video, or open the page's own menu.
6. On a site with a strict Content Security Policy (for example github.com) nothing breaks and the page works as before.

## Desktop app

The window cannot be built where the tests run (it needs the system web view), so its Rust shell (`app/src-tauri`) has only been checked by reading; please run these on Windows.

1. `cargo build --release -p mohona-cli`, then in `app`: `npm install`, set `MOHONA_PROGRAM` to the built `mohona.exe`, `npm run tauri dev`. The window opens and shows "Mohona is not running" with a "Start Mohona" button. If it does not compile, send the first error.
2. Press "Start Mohona": the banner goes away and "Engine running (0.0.1)" shows. `%LOCALAPPDATA%\Mohona\daemon.json` exists. Close the window: `mohona list` still works (the engine keeps running).
3. "+ Add", paste a real download address: the row appears within a second, with a moving progress bar, speed and time left. Pause and Resume (row buttons and toolbar) work; Cancel marks it cancelled.
4. Add the same address again: the dialog shows the reason it was refused. Add `example.com/a.zip` (no http): "the address must start with http://…".
5. Select a finished download, "Remove", tick "Also delete the downloaded file": the file is gone from the folder. Without the tick it stays.
6. Ctrl-click selects several rows; the toolbar acts on all of them.
7. Theme button cycles system, light, dark; the choice is remembered after restart. With "system", switching Windows between light and dark changes the window.
8. A download started from the browser extension appears in the window without any action.
9. Settings: set a speed limit of 200 KB/s and start a large download: the speed stays near the limit; clear the box: it speeds up. Set "Downloads at the same time" to 1 and add three downloads: one runs, two wait. Type a folder that does not exist (`D:\\Mohona test`) and Save: the folder is created and a new download goes there; a relative path (`downloads`) is refused with a reason.
10. Settings, browsers: add `example.com` to the sites list and Save; within a minute the extension's popup ("Which downloads to take over") shows it. Change something in the extension popup: reopening Settings in the app shows it.
11. Tray: the Mohona icon is in the notification area (maybe under the ^ arrow). Left click shows the window. Close the window: it disappears but the icon stays and downloads continue (`mohona list`). Settings, untick "Keep Mohona in the tray", Save, close the window: the app really exits and the engine keeps running (`mohona list` works). Tray menu "Quit and stop Mohona": the engine stops (`mohona list` says it is not running).
12. Messages: add a small download: a "Download finished" message appears (window open or hidden). Add a link that fails (a wrong path on a real server): "Download failed". Restart the app with old finished downloads in the list: no messages. Untick "Show a message…": none appear.
13. Finished row: "Open" opens a PDF or image with its program; "Show in folder" opens Explorer with the file selected. Download any `.exe` or `.zip`: "Open" on the `.exe` shows the safety message and does not start it. Delete the file in Explorer, press "Open": "the file is no longer there".
14. Schedule: open "Schedule", add a window that starts one minute from now (for example now is 14:20: from 14:21 to 14:50), Save. Add a large download: it waits ("queued") with the notice "Outside the scheduled hours"; at 14:21 it starts by itself; with a limit set on the window the speed follows that limit. Remove the window: it runs at any time again. A window from 23:00 to 06:00 is open at 02:00 and closed at 12:00.
15. End action, safe test: choose "Put the computer to sleep" with 10 seconds, Save, add a small download: when it finishes a banner counts down; press Cancel before it ends: nothing happens and the banner goes away. Real test (save your work first): repeat without cancelling: the computer goes to sleep (and for "Shut the computer down" shuts down). Hibernate needs hibernation to be enabled in Windows; when it is not, Mohona sleeps instead.
16. Folders and categories: in the Add dialog press "Browse…": Windows' folder dialog opens; the chosen folder appears in the box; a download added there lands in that folder. Cancelling the dialog leaves the box as it was. Same button in Settings for the default folder.
17. "Categories": the standard ones are listed. Add "Books" with `epub, mobi`: it appears. Add "Archives" with `zip`: refused with "already belongs to Compressed". A name like `a/b`, file types like `z!p`, or a folder like `..\\x` are refused with a reason. Remove "Books".
18. Sorting: tick "Sort finished downloads into folders by type" in Settings, download a `.zip` and a `.mp4` (also one from the browser): the files end up in `Compressed` and `Video` folders inside the download folder. Untick: new downloads stay in the download folder.
19. Queues: press "Queues", add "Night": it appears in the list. Adding "night" again is refused. Select Night, set "Downloads at once" to 1 and a speed limit of 200 KB/s, Save. In "+ Add" a "Queue" choice appears; add two large downloads to Night: one runs, one waits, and the speed stays near 200 KB/s while Main downloads are not limited. Give Night a window that is closed now: its downloads wait ("queued") while Main's run. Remove Night: its downloads continue in Main. Main has no working "Remove queue" button.
20. Details: start a large download and press "Details" (or double-click the row): totals, speed and time left update every second; a table lists the connections (usually 2 to 8) with their part of the file, their own progress bar and speed; the bar above it fills in several places at once. Pause the download: the window says the connections show while it is running. Add a small file: finished downloads have no Details button. While a download runs the progress bar must never move backwards.
21. Categories in the list: every row has a category label. Click "Compressed" (or any) in the sidebar: only its files show; click it again or "All types": all show. The numbers next to the categories follow the status filter ("Finished", ...). "Sort by": Name, Size, Category and type, Status each reorder the list; the choice is still there after restarting the app.
22. Themes: the box at the top right lists System, Light, Dark, Desert, Forest, Sunny, Orange, Leaf, Earth, Sky & cloud, River, Water. Each one changes the whole window at once, the header text is readable on every one, and the choice is still there after restarting.
23. `npm run tauri build` makes an installer in `app\src-tauri\target\release\bundle`. (The installer does not include `mohona.exe` yet: Phase 8.)

## Extension popup

1. Open the popup with Mohona running: the header shows "Mohona x.y.z is running" with a green dot; four counters show running, waiting, finished and failed downloads. Start a big download: it appears under "Downloads" with a progress bar, speed and time left, and "Total speed" shows; they update every second or two.
2. Stop Mohona (`mohona` daemon quit or the tray's Quit): the dot turns yellow, "Start Mohona" shows and works.
3. The theme box in the header offers the twelve themes; the popup changes at once and keeps the choice after closing it. Check at least one dark theme (Forest, Earth, River) and one light (Sunny, Water): all text readable.
4. Finished files appear under "Recently finished"; media found on a page under "Media on this page"; "Settings" opens the same options as before.

## Store packaging

1. `cd extension && npm run package`, unzip `dist/store/mohona-chromium-0.0.1.zip` into an empty folder and load it unpacked in Chrome: it loads without errors, shows the Mohona icon, and its id is different from the development id (it has no `key`), so the popup says not connected until the host is registered for that id.
2. Load the Firefox zip as a temporary add-on: no manifest warnings except possibly about `data_collection_permissions` on Firefox older than 140.
3. Open both manifests: no `key` in the Chromium one.

## Shared settings

1. Chrome and Edge both loaded, Mohona running. In Chrome's popup open "Which downloads to take over", set 5 MB and `example.com`, Save. Within a minute (or after closing and reopening Edge's popup) Edge shows the same values.
2. Stop Mohona, change the size in Chrome, start Mohona: after a minute the change reaches Edge.
3. `mohona-host` with a damaged value: not testable by hand; covered by the unit tests.

## Power and system behaviour

- [ ] `queue run --then sleep`, `hibernate`, `shutdown` really do that after the countdown (try shutdown last), and Ctrl-C in the countdown cancels.
- [ ] Put the computer to sleep during a download, wake it → `The network is back; continuing downloads...` and it finishes.
- [ ] Unplug the network cable / turn off Wi-Fi during a download, plug back → it continues.
- [ ] Kill test on Windows: add 20 downloads, `queue run -s 5`, close the window midway, run again → all complete, none lost (Linux automated test passes).
- [ ] `queue run --then run:notepad.exe` opens Notepad when done.

## Servers with limits

- [ ] A fast real server (not one that limits per address): `get URL -c 8` is faster than `-c 1`.
- [ ] `-l 500K` really holds the speed near 500 KB/s.

## Add new items here

When a step adds something that can't be tested on this PC (FTP server, SFTP server, WebDAV, a custom CA, a mirror site, a browser, an Android phone), add it to this list in the same step.
