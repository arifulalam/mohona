# Mohona (মোহনা)

**An estuary for your downloads.**

Mohona is a free, open-source download manager. It splits each file across many connections, resumes after any interruption, and brings streams, torrents and browser downloads into one place. It runs on Windows, macOS and Linux.

> **Status: early development (Phase 1).** Mohona can probe a URL and download it over several connections at once, re-balancing them as it goes. Resume after a failure, retries, queues and the rest are still to come. Nothing here is ready for everyday use yet.

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

Add `--connections 16` (or `-c 16`) to choose how many connections to use (1 to 32, default 8). A file is only split when the server supports byte ranges and the file is at least 1 MiB.

The file is written as `name.part` and renamed when it is complete, and an existing file is never overwritten (the new one becomes `name (1).ext`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in Mohona, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
