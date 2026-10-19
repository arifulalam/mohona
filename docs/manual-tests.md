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

## Logins and cookies against a real site

- [ ] A real HTTP Basic site: `get URL -u user:password` → downloads; wrong password → `refused the user name or password`.
- [ ] A real Digest site → downloads.
- [ ] https://www.trainedbyjp.com/downloads/4k_Thetestdata.mp4 after the server is fixed: `--cookie` with a real session cookie → downloads (earlier answer was 401, then 500 from the server).

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
