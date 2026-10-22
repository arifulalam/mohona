# Mohona command reference

Every command, with an example, what you should see, and what the common errors look like. Output below was captured from a real run; sizes, numbers and dates will differ on your machine.

**General**

- Run commands as `mohona ...` (or `cargo run -p mohona-cli -- ...` from the source folder).
- Every command takes `--db FILE` (default `mohona.db` in the current folder): the file that keeps the download list.
- A successful command exits with `0`; any `error:` line exits with `1`.
- `mohona --help` and `mohona <command> --help` always show the current options.

---

## `get` — download one URL right now

```
mohona get URL [-o FOLDER] [-c N] [-l SPEED] [-v] [request options]
```

| Option | Meaning |
|---|---|
| `-o, --output FOLDER` | Folder to save in (default: current folder) |
| `-c, --connections N` | Most connections at once, 1 to 32 (default 8) |
| `-l, --max-speed 500K` | Speed limit: `500K`, `1.5M`, `2G` or plain bytes |
| `-v, --verbose` | Show decisions (connections added/dropped, server pushback) |

**Request options** (also work on `queue add`):

| Option | Meaning |
|---|---|
| `-H "Name: value"` | Extra header; repeat for several |
| `--cookie "a=1; b=2"` | Send cookies |
| `--referer URL` | The page the link was on |
| `--user-agent TEXT` | Pretend to be a browser |
| `-u user:password` | Log in (Basic or Digest, when the server asks). Without `:password`, the `MOHONA_PASSWORD` environment variable is used. `user:pass@` in the URL also works |
| `--proxy URL` | `http://`, `https://`, `socks5://` or `socks5h://` proxy; put `user:password@` in it for a login |
| `--no-proxy` | Ignore every proxy, even the system one |
| `--mirror URL` | Another address of the same file; repeat for more. The download uses all addresses together and moves on when one fails (see below) |
| `--ca-file FILE` | Also trust the certificate authority in this PEM file (a server with its own certificate). The full path is saved with the download |
| `--insecure` | Accept any certificate (HTTPS, FTPS) or SSH host key (SFTP), even a wrong one. Unsafe: only for a server you know |
| `--key FILE` | SFTP: log in with this private key. A passphrase is read from the `MOHONA_KEY_PASSPHRASE` environment variable and is never saved |
| `--host-key SHA256:...` | SFTP: trust only the server with this key fingerprint |
| `--known-hosts FILE` | SFTP: the list of known servers (default `.ssh/known_hosts` in your home folder, as OpenSSH uses) |
| `--trust-new-host` | SFTP: remember a server seen for the first time. A server whose key *changed* is never accepted this way |

### Example: plain download

```
$ mohona get https://example.com/file.zip -o downloads
Downloading file.zip (1.9 MiB, up to 3 connections)
Saved downloads/file.zip (1.9 MiB)
```

If the name is taken, the new file gets ` (1)`, ` (2)`... and nothing is overwritten.

### Example: with a login

```
$ mohona get https://host/private/file.bin -u alice:secret -o downloads
$ MOHONA_PASSWORD=secret mohona get https://host/private/file.bin -u alice -o downloads
```

### Example: cookies, headers, browser name

```
mohona get URL --cookie "session=abc" --referer https://site/page --user-agent "Mozilla/5.0" -H "Accept-Language: bn"
```

### Example: through a proxy

```
mohona get URL --proxy http://host:3128
mohona get URL --proxy socks5h://user:password@127.0.0.1:1080
mohona get URL --no-proxy
```

`socks5h` lets the proxy look up the name; `socks5` looks it up on your computer.

### Example: a server with its own certificate

```
mohona get https://nas.local/file.zip --ca-file my-ca.pem
mohona get https://nas.local/file.zip --insecure
```

Certificates are checked by default. Prefer `--ca-file`: it still protects you against anyone else pretending to be the server. `--insecure` is for one download at a time and is never turned on by itself.

### Example: check the file after downloading

```
$ mohona get https://example.com/tool.iso --checksum sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 -o downloads
Downloading tool.iso (1.9 MiB, up to 3 connections)
Saved downloads/tool.iso (1.9 MiB)
Checked: sha256 matches (the checksum you gave)

$ mohona get https://example.com/tool.iso --find-checksum -o downloads
Checked: sha256 matches (https://example.com/tool.iso.sha256)
```

- `--checksum HASH` takes `sha256:HEX`, or just the hex (md5, sha1, sha256 and sha512 are told apart by length).
- `--find-checksum` looks for `<address>.sha256`, `.sha512`, `.sha1`, `.md5` next to the file (one hash, `hash  name`, or a list that names the file).
- Without either flag, a hash the server announces itself (`Content-MD5`, `Digest`, `Repr-Digest` headers) is checked when there is one. Nothing is checked when nothing is announced.
- A given checksum wins over a checksum file, which wins over the server's headers.
- A file that does not match is **kept** and the command fails, because the file may be what you want and re-downloading is expensive:

```
error: the file does not match its checksum: expected sha256:cdcd... but it is sha256:9f86.... The file was kept at downloads/tool.iso (it may be damaged, or the checksum may be wrong)
```

With `queue add`, `--checksum` works for one address at a time. The download ends as `failed` (it is not retried: the same bytes would come again).

### Example: FTP

```
mohona get ftp://ftp.example.org/pub/tool.iso -o downloads
mohona get ftp://ftp.example.org/private/file.zip -u alice:secret -o downloads
mohona queue add ftp://alice:secret@host/private/file.zip     # the login is not kept in the list
```

- With no login, Mohona signs in as `anonymous`.
- Resumable and split into pieces like HTTP, when the server supports `REST` (most do). Each piece uses its own connection; servers that allow only a few get fewer automatically (421).
- Needs a file address, not a folder. Passive mode only (the usual one behind routers).
- Not yet: FTP through a proxy (`--proxy` is ignored for FTP addresses), and listing folders.
- Works with `--checksum`, `--mirror` (also mixed with http addresses of the same size) and the queue.

### Example: encrypted FTP (FTPS)

```
mohona get ftps://host/file.zip -u alice:secret      # encrypted from the first byte (port 990)
mohona get ftpes://host/file.zip -u alice:secret     # starts plain, then switches with AUTH TLS (port 21)
mohona get ftpes://nas.local/file.zip --ca-file my-ca.pem
mohona get ftpes://nas.local/file.zip --insecure
```

- Both encrypt the login **and** the file data. A plain `ftp://` address never switches to encryption by itself: if your server offers encryption, use `ftpes://`.
- The certificate rules are the same as for HTTPS (`--ca-file`, `--insecure`; see above). The login is not sent until the connection is encrypted and the certificate accepted.
- Errors: a refused certificate shows the same message as for HTTPS. `ftpes://` against a server without encryption fails with `the FTP server answered 502/504 ...`; no password is sent.

### Example: SFTP

```
mohona get sftp://alice@host/home/alice/file.zip -u alice:secret --host-key SHA256:abc...
mohona get sftp://alice@host/data/file.zip --key ~/.ssh/id_ed25519 --trust-new-host
set MOHONA_KEY_PASSPHRASE=my passphrase        (Windows; use export on Linux/macOS)
mohona get sftp://alice@host/data/file.zip --key id_ed25519
```

- **A user name is required** (in the address or with `-u`). Without `--key`, a password is used if given; with neither, the keys `~/.ssh/id_ed25519`, `id_ecdsa` and `id_rsa` (no passphrase) are tried.
- **The server is checked before any password or key is offered.** It must be in your `known_hosts` file, or match `--host-key`, or be remembered with `--trust-new-host`. The first time, the error shows the fingerprint to compare with what the server's owner gives you:

```
error: the server's SSH key was not accepted: host is not a known server (its key is SHA256:abc...). If that is the key you expect, trust it with --host-key SHA256:abc..., or with --trust-new-host to remember it
```

- **If a known server's key changed**, the download stops with a warning and `--trust-new-host` does not override it; check with the server's owner, then fix your known_hosts file.
- Split into pieces and resumable like HTTP and FTP. Each piece uses its own SSH connection; servers that limit connections (`MaxSessions`, `MaxStartups`) get fewer automatically.
- Works with `--checksum`, `--mirror` and the queue. Passwords given with `-u` are saved with a queued download like other logins; **key passphrases are not saved** (set the variable again when you run the queue).
- Not yet: SSH agent, `~/.ssh/config` (hosts aliases, ProxyJump), SFTP through a proxy, folder listings.

### Example: WebDAV

```
mohona get webdavs://cloud.example.com/remote.php/dav/files/alice/report.pdf -u alice:app-password
mohona get dav://nas.local/share/video.mkv -u alice
```

- `webdav://` (also `dav://`) is `http://`, and `webdavs://` (also `davs://`) is `https://`. Everything that works for HTTP(S) works the same: Basic and Digest logins, cookies, proxies, certificates (`--ca-file`, `--insecure`), pieces, resume, mirrors, checksums, the queue.
- Plain `http(s)://` addresses of WebDAV servers work too; the `webdav` names only say what the address is.
- Nextcloud/ownCloud: use an **app password** (Settings, Security) with `-u`, not your main password.
- Not yet: listing folders (`PROPFIND`), downloading whole folders.

### Example: mirrors

```
mohona get https://main.example.com/tool.iso --mirror https://mirror1.example.org/tool.iso --mirror https://mirror2.example.net/tool.iso --checksum sha256:... -o downloads
```

- The pieces of the file are spread over all addresses, so you get the speed of several servers at once.
- An address that fails, or keeps failing, is dropped and its pieces go to another one. If the main address is down at the start, the first mirror that answers takes its place.
- A mirror is used only if it has the **same size** and can send parts of the file. Others are skipped with a note (`the mirror ... is not used: it has a different size (...)`). Same size does not prove same content, so add `--checksum` when you can.
- If the file cannot be sent in pieces, mirrors are not used (a note says so).
- Mirrors are saved with a queued download (`queue add URL --mirror ...`).
- Every address uses the same cookies, login, proxy and certificate settings.

### Errors you may see

| Message | What it means / what to do |
|---|---|
| `the server answered 401 ...: it asks for a login (Basic)` | Add `-u user:password` |
| `the server answered 401 ...: it refused the user name or password` | Wrong login |
| `the server answered 401` with a note that it sent no login method | The site uses a web login page, not HTTP login. Log in in a browser and pass its cookie with `--cookie` |
| `the server answered 404 ...` | The address is wrong or the file is gone |
| `the server answered 500 ...` | The server itself failed; try later |
| `unsupported URL scheme "gopher" (only http, https, webdav, ftp, ftps, ftpes and sftp work so far)` | That kind of address is not supported |
| `SFTP login: the server refused the login for alice (wrong password or key?)` | Wrong password or key, or the account does not allow SFTP |
| `SFTP login: no login to offer: give -u user:password, or --key FILE` | No password was given and no usable default key was found |
| `SFTP login: SFTP needs a user name: ...` | Write `sftp://user@host/path` or use `-u` |
| `SFTP login: cannot use the key file ...: ... (if the key has a passphrase, set MOHONA_KEY_PASSPHRASE)` | Key file unreadable, not a key, or a wrong/missing passphrase |
| `the server's SSH key was not accepted: ...` | See the SFTP section: unknown server, changed key, or fingerprint mismatch |
| `SFTP: /path: the file is not there` / `you may not read it` | Wrong path or no permission. Not retried |
| `SSH problem: ...` | The SSH handshake or SFTP start failed (server down, no SFTP subsystem, incompatible algorithms) |
| `the FTP server answered 530: ... (it refused the login: give one with -u user:password)` | The FTP server wants a real login |
| `the FTP server answered 550: ... (the file is not there, or you may not read it)` | Wrong path or no permission. Not retried |
| `the FTP server answered 421: ... (the server is busy or limits connections: try fewer with -c)` | Too many connections to that server; Mohona already lowers them by itself, but `-c 2` helps up front |
| `connection problem: ...` | The server is not reachable (down, wrong port, firewall) |
| `invalid URL: ftp://host/dir/ (the address must name a file, not a folder)` | FTP addresses must name one file |
| `invalid proxy: "ftp://x": use http://, https://, socks5:// or socks5h://` | Fix the proxy address |
| `the argument '--no-proxy' cannot be used with '--proxy <URL>'` | Use one or the other |
| `invalid header: "Bad Header" is not like Name: value` | Headers need a colon |
| `the server's certificate cannot be trusted: it was not signed by an authority this computer trusts ...` | Use `--ca-file` with the server's authority, or `--insecure` if you accept the risk |
| `... : it has expired` / `it is for a different name than the one in the address` / `it is not valid yet (is this computer's clock right?)` | The server's certificate has a real problem; tell its owner. A wrong clock on your PC causes "not valid yet" |
| `cannot use the certificate file X: ...` | The `--ca-file` is missing or is not a PEM certificate |
| `invalid checksum "...": ...` | The `--checksum` value is not hex, or has the wrong length for its algorithm |
| `the file does not match its checksum` | The file is damaged, the checksum is wrong, or someone changed the file. The file is kept; compare it with the source |
| `--checksum is for one file; add the addresses one at a time...` | You gave `--checksum` with several addresses to `queue add` |
| `network error: ... Connection refused` | The server (or your proxy) is not listening there |
| `proxy asked for a login` (407) | Put `user:password@` in the `--proxy` address |

Many connections can make some servers time out (some limit one address to a few). Try `-c 2`.

---

## `stream` — download an HLS (.m3u8) or DASH (.mpd) stream

```
mohona stream <URL> [-q best|worst|audio|720] [--audio en] [--list] [--no-join] [--subs LANGS] [--embed-subs] [--container mp4|mkv] [--ffmpeg PATH] [-o FOLDER] [-n NAME] [-c N] [-l SPEED] [request options]
```

The address can be a playlist or manifest; Mohona tells HLS and DASH apart by reading it. The request options are the same as for `get` (`-H`, `--cookie`, `--referer`, `--user-agent`, `-u`, `--proxy`, `--no-proxy`, `--ca-file`, `--insecure`), so a stream behind a login, cookie or proxy works the same way. `--checksum`, `--mirror` and the SFTP options are accepted but do nothing for streams.

| Option | Meaning |
|---|---|
| `-q`, `--quality` | `best` (default), `worst`, `audio` (sound only), or a height like `720` or `720p`: the best one *not taller* than that (if all are taller, the smallest). |
| `--audio` | Audio language or name (like `en` or `Bangla`) when the stream has several. Default: the one marked default, else the first. |
| `--list` | Show what the stream offers and exit without downloading. |
| `--no-join` | Keep video and audio as two files; do not run ffmpeg. |
| `--subs` | Also download subtitles: a language like `en`, several like `en,bn`, or `all`. Each is saved as `NAME.LANG.srt`. |
| `--embed-subs` | Also put the subtitles into the finished video as tracks (needs `--subs` and ffmpeg). The `.srt` files stay. |
| `--container` | File type to join into: `mp4` (default) or `mkv`. |
| `--ffmpeg` | Where ffmpeg is (the program, or the folder holding it). Default: the `MOHONA_FFMPEG` variable, next to `mohona`, then the `PATH`. |
| `-o`, `--output` | Folder to save in (default: the current folder). |
| `-n`, `--name` | File name without extension (default: from the address). |
| `-c`, `--connections` | Segments fetched at once, 1 to 32 (default 8). |
| `-l`, `--max-speed` | Speed limit, like `500K` or `1.5M`. |

### Example: see what is offered

```
$ mohona stream https://example.com/video/master.m3u8 --list
HLS stream with 3 qualities:
  720p 2.5 Mbit/s  [avc1.4d401f,mp4a.40.2]
  360p 0.8 Mbit/s  [avc1.42c01e,mp4a.40.2]
  0.1 Mbit/s  [mp4a.40.5]  (sound only)
Audio: en (default), bn
```

For DASH the list shows one line per video, audio and subtitle stream, each with its segment count.

### Example: download

```
$ mohona stream https://example.com/video/master.m3u8 -o videos -q 360
Quality: 360p 0.8 Mbit/s  [avc1.42c01e,mp4a.40.2]
Audio: en
Downloading video ...
  3/3 segments, 1.8 KiB
Saved videos/master.ts (1.8 KiB)
Downloading audio ...
  2/2 segments, 610 B
Joining video and audio with ffmpeg ...
Saved videos/master.mp4 (2.4 KiB)
```

When the audio is separate from the video (most DASH streams, many HLS ones) both are downloaded and then joined into one file by **ffmpeg**, which only copies the streams (no re-encoding, no quality loss, fast). The two downloaded files are removed only after the join worked.

If ffmpeg is not installed, nothing fails: you keep `NAME.EXT` (video) and `NAME.audio.EXT` (audio), and Mohona says how to install ffmpeg or point to it. To join them later without downloading again: `ffmpeg -i NAME.mp4 -i NAME.audio.m4a -map 0:v:0 -map 1:a:0 -c copy joined.mp4` (use the real file names). `--no-join` keeps two files on purpose. The extension of the unjoined files is `.ts` for MPEG-TS segments and `.mp4` / `.m4a` for fragmented MP4.

### Subtitles

```
$ mohona stream https://example.com/video/master.m3u8 --subs en,bn --embed-subs
...
Saved videos/master.mp4 (2.4 KiB)
Downloading subtitles en ...
Saved videos/master.en.srt
Downloading subtitles bn ...
Saved videos/master.bn.srt
Adding the subtitles to the video with ffmpeg ...
Saved videos/master.mp4 (with subtitles)
```

Subtitles in WebVTT (the usual HLS kind, even when split over many small files) and TTML are converted to SRT, which every player understands. `--list` shows which languages a stream has. With `--embed-subs` they also become selectable tracks inside the MP4 (or MKV) with the right language tag. Embedding only works on one joined video file: with `--no-join`, or if ffmpeg is missing, you keep the `.srt` files and a message says so.

Not supported yet: subtitles stored *inside* MP4 segments (`stpp` or `wvtt` in DASH, rare) give the message "these subtitles are neither WebVTT nor TTML text".

### Stop and resume

Press Ctrl-C at any time. Finished segments are kept in a `NAME.EXT.parts` folder beside the result. Run the same command again and only the missing segments are fetched. When everything is there the pieces are joined and the folder is removed.

### Errors you may see

| Message | What it means and what to do |
|---|---|
| `"huge" is not a quality (use best, worst, audio, or a height like 720)` | Fix the `-q` value. |
| `the server answered 404 for ...` | The playlist address is wrong or expired (many sites give out short-lived addresses). Get a fresh one. |
| `... is not a stream: it is neither an HLS playlist nor a DASH manifest` | The address gave a web page or something else. Look for the real `.m3u8` or `.mpd` address (browser developer tools, Network tab). |
| `this stream is protected (...); DRM-protected streams are not supported` | The stream uses DRM (Widevine, PlayReady, FairPlay, SAMPLE-AES). Mohona cannot and will not download it. |
| `this is a live DASH stream; live streams are not supported yet` | Live recording comes later in Phase 4. |
| `the encryption key at ... is N bytes, not 16 (the server may require a login or cookie)` | The key address returned a page instead of a key. Add `--cookie`, `-H` or `-u` the way the site needs. |
| `segment N could not be decrypted (wrong key or damaged data)` | The key does not match. Try again later; if it repeats the stream may be using a scheme Mohona does not support. |
| `no subtitles in \"fr\" (this stream has: en, bn)` | Use one of the listed languages, or `--subs all`. |
| `ffmpeg could not add the subtitles: ...` | The video is unchanged and the `.srt` files are kept. Try `--container mkv`. |
| `ffmpeg could not join the files: ...` | ffmpeg refused the two files (often a codec the chosen container cannot hold). The two files are kept; try `--container mkv`, or join by hand. |
| `Video and audio are saved as two files because ffmpeg was not found.` | Not an error. Install ffmpeg (Windows: `winget install ffmpeg`) or use `--ffmpeg PATH`. |
| `the playlist lists no segments` | Nothing to download (for example an empty or ended live playlist). |

## `queue` — a saved list of downloads

The list lives in the `--db` file, so it survives closing the window, a crash, or a restart. Downloads continue where they stopped.

### `queue add`

```
mohona queue add URL [URL...] [-o FOLDER] [--sort] [--duplicates ask|rename|skip|overwrite] [request options]
```

```
$ mohona queue add https://example.com/a.zip https://example.com/b.zip -o downloads --sort
#1 added: https://example.com/a.zip
#2 added: https://example.com/b.zip
```

- `--sort` moves finished files into the folder of their category (see `queue categories`).
- Request options (`-H`, `--cookie`, `--referer`, `--user-agent`, `-u`, `--proxy`, `--no-proxy`) are saved with each download.

Duplicate (default `ask`):

```
$ mohona queue add https://example.com/a.zip
https://example.com/a.zip: already in the list as #1 (queued). Use --duplicates rename, skip or overwrite.
error: 1 not added
```

`rename` adds it with a new file name, `skip` ignores it, `overwrite` replaces the old file.

### `queue list`

```
$ mohona queue list
#1   completed   97.7 KiB of 97.7 KiB     a.zip
#2   queued      0 B                      https://example.com/b.zip
```

States: `queued`, `downloading`, `retrying`, `paused`, `failed`, `canceled`, `completed`.

### `queue run`

```
mohona queue run [-s N] [-l SPEED|off] [--then ACTION] [--countdown SECONDS]
```

```
$ mohona queue run -s 2
#1 downloading a.zip
#2 downloading b.zip
#2 done: downloads/b.zip
#1 done: downloads/a.zip
```

- `-s N` how many at once (saved for next time). `-l` total speed limit (saved).
- Ctrl-C stops and keeps progress; run it again to continue.
- `--then exit|sleep|hibernate|shutdown|run:<program> [args]` does that when everything is done, after a countdown (default 60 s). Ctrl-C during the countdown cancels the action.
- If the network drops: `The network changed...` then `The network is back; continuing downloads...`.
- Timeouts show `retrying 1/2` and continue by themselves.

### `queue schedule`

```
$ mohona queue schedule
The queue may run at any time.
$ mohona queue schedule --days mon-fri --from 01:00 --to 07:00 --speed 2M
```

With no options it shows the current schedule.

### `queue speed`, `queue retry`, `queue remove`

```
mohona queue speed 3 500K     # limit download #3 (or: off)
mohona queue retry 3          # put a paused/failed/canceled one back
mohona queue remove 3         # take it off the list
$ mohona queue remove 99
error: no download #99
```

### `queue categories`

```
$ mohona queue categories
#1 Compressed -> Compressed  (zip, rar, 7z, gz, tar, bz2, xz, zst)
#2 Documents -> Documents  (pdf, doc, docx, ...)
...
$ mohona queue categories add Data dat,bin --folder Data
#6 Data added
$ mohona queue categories remove 6
```

---

## `history` — search everything ever added

```
mohona history [WORDS...] [--status S] [--since YYYY-MM-DD] [--until YYYY-MM-DD] [--category NAME] [-l N]
```

```
$ mohona history zip --status completed --since 2026-10-01
#2   2026-10-19 10:25  completed    195.3 KiB  b.zip
```

Each word must match the start of a word in the name, address or category. Nothing matching prints `Nothing found.`

## `log` — what happened to one download

```
$ mohona log 1
2026-10-19 10:25  info  added
2026-10-19 10:25  info  completed: downloads/a.zip (100000 bytes)
```

Errors: `error: no download #99`. A download added long ago may say `Nothing logged for #N` if it was added before logging existed.

## `network` — what Mohona watches

```
$ mohona network
IPv4 route uses: 192.168.1.20
IPv6 route uses: none
Online.
```

When the address changes while `queue run` is going, downloads pause and continue when the network is back.
