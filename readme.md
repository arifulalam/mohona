# Mohona (মোহনা)

**An estuary for your downloads.**

Mohona is a free, open-source download manager. It splits each file across many connections, resumes after any interruption, and brings streams, torrents and browser downloads into one place. It runs on Windows, macOS and Linux.

> **Status: early development (Phase 2).** Mohona can probe a URL and download it over several connections at once, re-balancing them as it goes. It measures each server to find out how many connections actually help, retries failed requests, and backs off when a server limits connections. If a download is interrupted (Ctrl-C, a crash, a power cut, a dropped network), run the same command again and it continues where it stopped. Phase 2 is under way: `mohona queue add/list/run` keeps a download list in a SQLite database, limits how many run at once, and keeps its progress in the database, retries on network errors by itself, and picks up where it stopped after a restart or crash. Speed limits work at three levels (`-l 1M` on `get`, `queue run -l`, `queue speed <id> 500K`). `queue schedule --days mon-fri --from 01:00 --to 07:00 --speed 2M` limits when the queue runs. `queue run --then shutdown --countdown 60` runs an action when everything is done, with a countdown you can cancel. The rest of Phase 2 is still to come. Nothing here is ready for everyday use yet.

## Goals

- Everything a classic download manager does: segmented downloads, resume, queues, scheduler, speed limiter, browser capture, video grabber, site grabber
- And more: HLS and DASH streams, SFTP and WebDAV, BitTorrent, a scriptable CLI and local API, a rules engine, checksum verification, dark mode
- Reliable: a crash, power cut or network switch never corrupts a download

## Repository layout

| Path | What it is |
| --- | --- |
| `crates/core` | The download engine (segments, workers, retries) |
| `crates/net` | Transports: HTTP, FTP, SFTP, WebDAV, proxies, authentication |
| `crates/store` | SQLite storage for downloads, queues and history |
| `crates/cli` | The `mohona` command-line program |
| `tools/testserver` | Local server that serves generated files and misbehaves on purpose, for tests |
| `docs/` | Decisions and notes |

## Build and test

You need [Rust](https://rustup.rs) (stable). On Windows, Rust also needs the Visual Studio C++ Build Tools.

```
cargo test --workspace
cargo run -p mohona-cli -- --version
```

### Try the test server

```
cargo run -p mohona-testserver
```

It listens on `http://127.0.0.1:8080` and prints its own help. Examples:

```
curl -I "http://127.0.0.1:8080/file"                         # 1 GiB file, headers only
curl -r 100-199 "http://127.0.0.1:8080/file" | wc -c          # exactly 100 bytes
curl "http://127.0.0.1:8080/file?drop_after=5000" | wc -c     # connection cut after 5000 bytes
```

### Download a file

With the test server running in another window:

```
cargo run -p mohona-cli -- get "http://127.0.0.1:8080/file?size=50000000" --output downloads
```

Add `-v` to see what Mohona decides about connections. `--connections 16` (or `-c 16`) sets the most connections Mohona may use (1 to 32, default 8). It often uses fewer: it measures the server and only keeps connections that make the download faster. A file is only split when the server supports byte ranges and the file is at least 1 MiB.

While a download runs it writes `name.part` and a small progress file `name.part.mohona`. Press Ctrl-C to stop; run the same command again to resume. The file is written as `name.part` and renamed when it is complete, and an existing file is never overwritten (the new one becomes `name (1).ext`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in Mohona, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

### Categories and duplicates

```
mohona queue categories add Video mp4,mkv,webm --folder Videos
mohona queue categories
mohona queue add <url> -o downloads --sort            # finished files go to their category folder
mohona queue add <url> --duplicates rename|skip|overwrite   # default: ask
```

### History and search

```
mohona history ubuntu                       # name, address or category; prefixes work
mohona history --status failed --since 2026-10-01 --until 2026-10-17
mohona history --category Video -l 20
mohona log 3                                # what happened to download #3
```

### Network changes

`queue run` notices a Wi-Fi switch, a VPN going up or down, or the network going away: it pauses the downloads, keeps their progress, and continues when the new network is steady. `mohona network` shows the addresses it watches.

### Cookies, referrer and headers

For links that only work from a logged-in page:

```
mohona get <url> --cookie "session=abc" --referer https://example.org/page -H "X-Token: abc"
mohona queue add <url> --cookie "session=abc" --referer https://example.org/page
```

The details are saved with the download (in plain text, in the database) so retries and restarts send them again.

### Logging in

```
mohona get https://host/file.zip -u user:password        # Basic or Digest, whichever the server asks for
MOHONA_PASSWORD=secret mohona get https://host/file.zip -u user
mohona queue add https://user:password@host/file.zip      # the login is taken out of the address
```

The password is only sent when the server asks for it, and only to that server. It is kept in the database in plain text for now.

### Proxies

```
mohona get URL --proxy http://host:3128
mohona get URL --proxy socks5h://user:password@127.0.0.1:1080
mohona get URL --no-proxy          # ignore any system proxy
mohona queue add URL --proxy ...   # saved with the download
```

`socks5h` lets the proxy look up the name; `socks5` looks it up here. Without either flag, the system proxy is used.

### Certificates

HTTPS certificates are checked. For a server with its own certificate:

```
mohona get https://host/file --ca-file my-ca.pem   # trust this authority too
mohona get https://host/file --insecure            # skip the check (unsafe)
```

### Checksums

```
mohona get URL --checksum sha256:ABC...    # check against a hash you have
mohona get URL --find-checksum             # use URL.sha256 (or .sha512/.sha1/.md5) if it exists
```

Hashes the server announces itself are checked automatically. A file that does not match is kept, and the download is reported as failed.

### Mirrors

```
mohona get URL --mirror URL2 --mirror URL3 --checksum sha256:...
```

The pieces of the file come from all addresses at once; a failing address is replaced by another. Only mirrors with the same size are used; add `--checksum` to be sure the content is the same.

### FTP

```
mohona get ftp://host/pub/file.iso
mohona get ftp://host/private/file.zip -u alice:secret
```

Use `ftps://` (encrypted from the start) or `ftpes://` (switches to encryption) for encrypted FTP; the certificate rules are the same as for HTTPS. Anonymous by default. Downloads are split into pieces and can be resumed, as with HTTP, when the server allows it.

### SFTP

```
mohona get sftp://alice@host/path/file.zip -u alice:secret --trust-new-host
mohona get sftp://alice@host/path/file.zip --key ~/.ssh/id_ed25519 --host-key SHA256:...
```

The server's key is checked against your `known_hosts` (or a fingerprint you give) before any password or key is sent.
