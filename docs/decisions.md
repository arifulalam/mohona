# Decisions

Short records of choices that shape Mohona. Add a new entry when a decision changes.

## ADR-0001: Rust for the engine, TypeScript for the interface

- **Date:** 2026-10-14
- **Decision:** The engine, CLI and native messaging host are written in Rust. The desktop app (Tauri 2) and the browser extension use TypeScript with React, Vite and Tailwind CSS.
- **Why:** One small native binary per operating system, no runtime to bundle, safe concurrency for many connections, and a web-technology interface.
- **Cost:** Rust has a steeper learning curve than Python, so we build in small, testable steps and keep every step runnable from the CLI.

## ADR-0002: Name and tagline

- **Date:** 2026-10-14
- **Decision:** The project is called **Mohona** (মোহনা, Bengali for estuary), with the tagline "An estuary for your downloads".
- **Why:** An estuary is where many rivers meet the sea, like many segments merging into one finished file. The plain English name "Estuary" was rejected because an established data-integration company already uses it.
- **Open:** Check crates.io, npm, GitHub, a domain and a trademark search before the first public release.

## ADR-0003: License is MIT OR Apache-2.0

- **Date:** 2026-10-14
- **Decision:** Mohona is dual licensed under MIT or Apache-2.0, at the user's option. This is the convention in the Rust ecosystem.
- **Why:** Both are permissive, so anyone can use and build on Mohona. Apache-2.0 adds an explicit patent grant, and MIT is short and widely understood. Dual licensing keeps Mohona compatible with most other projects.
- **Note:** ffmpeg runs as a separate program (not linked into Mohona), so its license does not change ours. Revisit in Phase 8 when deciding how ffmpeg ships.
- **Not legal advice.** Check with a lawyer if the project becomes commercial.

## ADR-0004: Hand-written test server

- **Date:** 2026-10-14
- **Decision:** `tools/testserver` is a small hand-written HTTP/1.1 server on tokio, not a web framework.
- **Why:** A download manager must be tested against unusual behavior: ignored Range headers, connections cut mid-body, throttled speeds, changing ETags. Writing the protocol by hand gives exact control, and generating file content from the byte offset means a 1 GiB test file needs no disk space.

## ADR-0005: HTTP/1.1 only, TLS through rustls

- **Date:** 2026-10-14
- **Decision:** The HTTP client (reqwest) is built without HTTP/2. TLS uses rustls with the operating system's certificate store. On Windows the build enables prebuilt NASM objects for `aws-lc-rs`, so no extra tools are needed beyond the C++ Build Tools.
- **Why:** Segmented downloads need several separate connections to one server, and HTTP/2 would multiplex them onto one. We will benchmark both in Phase 3, step 6.
- **Open:** The test server speaks plain HTTP only, so HTTPS is not covered by automated tests yet. Test it by hand against a real site until we add an HTTPS test server.

## ADR-0006: Partial files are deleted on failure, for now

- **Date:** 2026-10-14
- **Decision:** If a download fails, its `.part` file is removed.
- **Why:** Resuming needs a record of which byte ranges are complete, and that ledger arrives in Phase 1, step 6. Until then a leftover `.part` file could not be trusted. From step 6 on, failed downloads keep their partial data.

## ADR-0007: One scheduler decides who downloads which bytes

- **Date:** 2026-10-15
- **Decision:** The file is created at full size, cut into equal ranges, and each range gets its own connection and file handle. A single `Scheduler` (behind one lock) owns the ranges. Workers ask it before writing each chunk, and a worker that finishes asks it to cut the biggest unfinished range in half. Pieces smaller than 512 KiB are never split further.
- **Why:** With one place that knows who owns which byte, two workers can never write the same byte, and a slow connection never leaves the others idle at the end. A test with random use checks that the ranges always tile the file exactly.
- **Safety:** A range request is sent with `If-Range` (the ETag). If the server answers anything but `206` at the right offset (file changed, ranges ignored), the download fails instead of writing wrong bytes.

## ADR-0008: Retry small failures, back off when the server pushes back

- **Date:** 2026-10-15
- **Decision:** Errors are sorted into three kinds (`Failure` in `mohona-net`). *Fatal* (404, file changed, bad URL, disk errors) stops the download. *Transient* (dropped connection, 500, 502, 504, timeouts mid-body) makes a worker wait and continue from the byte where it stopped, up to 6 failures in a row without progress. *Throttled* (429, 503, refused or timed-out new connections) makes the worker hand its segment back; the download lowers its connection count to the number that still work, and the segment is taken up again after a pause. Waits are exponential with random jitter, capped at 30 s, and `Retry-After` is honored.
- **Why:** Found on a real server (proof.ovh.net): 2 or more connections from one address got 429 or silent connect timeouts, and the first version of Mohona failed the whole download. Real servers limit connections per client, so backing off is normal operation, not an error.
- **Measuring instead of guessing:** More connections are not always faster. On a real server (proof.ovh.net) one connection ran at 1.4 MiB/s while 16 together crawled at 167 KiB/s. So a download starts with one connection and the `tuner` (in `mohona-core`) measures the total speed, adds ONE connection, waits for the speed to settle, and keeps it only if the total rose clearly (at least 50% for the second connection, down to 8% for later ones). Otherwise it asks the extra workers to stop (their segments are handed back, nothing is lost) and stops trying. Servers that limit each connection get the full benefit of parallelism; servers that punish it get one or two connections.
- **Patience (learned from a real log):** The first tuner doubled connections after a 1.5 s baseline. On proof.ovh.net the first connection was still in TCP slow start (253 KiB/s, later 1.4 MiB/s), so every extra connection looked like a big gain; the tuner went to 4, then 8 connections within 6 s, the server answered 429, and the download ended up three times slower than one plain connection. Now the first measurement waits about 2 s (8 samples) and averages 1.5 s, connections are added one at a time, and a 429 is followed by a longer pause.
- **One request first:** An adaptive download starts as a single request for the whole file, like a browser, and the tuner cuts it up only when more connections prove worth it. The first version pre-cut the file into 16 pieces, which meant a new request and TCP connection per piece even with one connection in use, and that fails on servers that ignore new connections. A connection attempt that is stuck waiting can be cancelled when the tuner decides it is not worth it.
- **Diagnosing real servers:** `mohona get -v` prints what the download decides and why (connections added or dropped, server pushback), so a failing run on a real server can be understood from its output.
- **Limits for now:** The connection count is decided once per download, from the first few seconds, and is not remembered per server (planned for Phase 3). Reaching full speed on a fast, per-connection-limited server takes a few seconds of measuring. The single-stream path (no range support) does not retry, since it cannot resume.

## Planned crates (checked 2026-10-14)

Each crate was looked up on crates.io on the date above. Versions can change, so Cargo.lock is the source of truth once a crate is added.

| Crate | Latest version | Last release | Used for |
| --- | --- | --- | --- |
| tokio | 1.53.3 | 2026-10-13 | Async runtime |
| reqwest | 0.13.5 | 2026-09-08 | HTTP and HTTPS client |
| rusqlite | 0.40.2 | 2026-08-08 | SQLite storage |
| russh | 0.64.1 | 2026-10-05 | SSH for SFTP |
| russh-sftp | 3.0.1 | 2026-09-28 | SFTP client |
| suppaftp | 12.2.0 | 2026-10-11 | FTP and FTPS |
| librqbit | 9.0.1 | 2026-08-20 | BitTorrent |
| axum | 0.8.9 | 2026-04-14 | Local API |
| clap | 4.6.7 | 2026-09-14 | Command-line parsing |
| m3u8-rs | 6.0.1 | 2026-07-30 | HLS playlists |
| dash-mpd | 0.21.0 | 2026-10-04 | DASH manifests |
| tauri | 2.12.3 | 2026-10-10 | Desktop app shell |

All twelve have had a release since April 2026. librqbit is the smallest project (about 64,000 recent downloads), so we will re-check it before Phase 7.
