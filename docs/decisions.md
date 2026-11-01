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

## ADR-0006: Partial files are deleted on failure, for now (superseded by ADR-0009)

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

## ADR-0009: A progress ledger makes downloads resumable

- **Date:** 2026-10-15
- **Decision:** A download that uses byte ranges writes `<name>.part` and a small ledger `<name>.part.mohona` (JSON). The ledger lists every segment as `(start, committed, end)`: the segment covers `start..end` and `start..committed` is on disk. If the download fails or is stopped (Ctrl-C, or `CancelToken`) after saving some data, both files are kept. The next run with the same URL and folder finds them, probes the server again, and continues if the size and `ETag`/`Last-Modified` still match; otherwise the old data is deleted and it starts over. Downloads without range support cannot resume and still delete their partial file on failure.
- **Crash safety:** Workers collect bytes in a buffer and mark them committed only after writing them. A checkpoint (every second, at the start, and on stopping) takes the list of committed bytes first, then flushes the `.part` file to disk, then replaces the ledger atomically (temporary file, `fsync`, rename). So the ledger never claims bytes that might not be on disk, and a power cut leaves either the old ledger or the new one. Before the final rename the file is flushed again.
- **Found by testing:** The first version wrote its first checkpoint after 2 s. Killing the program (`kill -9`) earlier left a `.part` file with no ledger, which the next run could not find, so it started fresh under a new name and left the orphan behind. The ledger is now written before any data arrives. A test that kills the real program 14 times at random moments and resumes each time ends with a file identical to a clean download.
- **Safety details:** The file name in the ledger is sanitized again on resume (a ledger is a file anyone can edit). A new download never picks a name whose `.part` file exists. Partial files from the file changing on the server (`If-Range` mismatch) are deleted, since they are useless.
- **Limits for now:** Two Mohona processes downloading the same URL into the same folder are not guarded against (a lock file is planned). A `.part` file with no readable ledger is left alone, since it may belong to something else. When a failed run's worker handles are still closing on Windows, deleting the partial files can fail and leave them behind.

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

## ADR-0010: SQLite schema and migrations

Status: accepted (16 October 2026)

The database is a single SQLite file in WAL mode, opened through `rusqlite` with the bundled SQLite (no system library needed on Windows). The schema follows the plan: downloads, segments, queues, categories, settings, log. The segments table replaces the Phase 1 sidecar ledger later in Phase 2.

Versioning uses SQLite's `user_version`. Each numbered SQL file in `crates/store/src/migrations` runs in its own transaction, so a failed upgrade leaves the previous version intact. A database from a newer Mohona is refused rather than modified. Foreign keys are on; deleting a download removes its segments and log. A default queue ("Main", 3 at once) and five default categories are created by migration 1.

## ADR-0011: The download manager

Status: accepted (16 October 2026)

`mohona-manager` runs one long-lived task (an actor) that owns every download. Front ends hold a cloneable `Manager` handle, send commands (add, pause, resume, cancel, remove, list, set queue limit, shutdown) over a channel and watch a broadcast stream of events. The database is written before an event is published, so it is always the truth; events are only a convenience and slow receivers may miss them.

- **States.** queued, probing, downloading, paused, retrying, completed, failed, canceled. `retrying` exists in the schema but is not used yet: the engine retries inside one download and has no event for it. Every change is saved.
- **Queues.** After every change the manager asks the database which queued downloads fit in the free slots of each queue (by position) and starts them. A queue never runs more than its `max_active`.
- **Pause and cancel.** Both stop the engine through its cancel token. Pause keeps the partial data and the `.part.mohona` ledger. Cancel deletes them (`discard_partial`). Removing a running download cancels it first.
- **Shutdown and crashes.** Shutdown stops everything cleanly and marks running downloads queued again. After a crash or kill the same is done at start-up (`reset_interrupted`). They resume from their ledgers.
- **Progress** is written to the database about once a second and when a download stops.
- **File names.** Two downloads with the same file name in one folder used to be able to pick the same name at the same moment. The engine now claims a name by creating its `.part` file with "create new" in the same step, so this is safe even across processes.
- **Not done yet:** speed limits, scheduling, duplicate detection (see ADR-0012 for progress storage and automatic retries).

## ADR-0012: Progress in the database, and automatic retries

Status: accepted (16 October 2026)

**Progress storage.** The engine asks a `ProgressStore` (a small trait in `mohona-core`: load, save, clear) where to keep a download's progress. With none given, it uses the sidecar file `<name>.part.mohona` as before, so `mohona get` is unchanged. The manager gives each download a store that writes to the `segments` table (and the size, `ETag` and `Last-Modified` columns of the `downloads` row) in one transaction. The checkpoint order is unchanged: snapshot the segments, flush the `.part` file, then save, so saved progress never claims bytes that are not on disk. Each store opens its own database connection, because workers save from other threads (WAL mode allows this). `mohona-core` does not depend on `mohona-store`.

A download that has a sidecar file from an older version still resumes: the engine falls back to the sidecar when the database has nothing, and removes the sidecar when the download ends.

**Automatic retries.** A download that fails with a network error that may pass (anything but a fatal one, such as a 404) goes to `retrying` and is started again after 5 s, 10 s, 20 s, ... up to 5 minutes, at most 5 times in a row. The count starts over when an attempt downloaded new bytes. While waiting it keeps its slot in the queue and can be paused or canceled. After the last retry it is marked failed; resuming by hand starts the count over. A failure that is not temporary (a missing file, a full disk) fails at once.

## ADR-0013: Speed limits

Status: accepted (16 October 2026)

A `RateLimit` (in `mohona-core`) is a shared pacing object: bytes per second, `0` meaning unlimited, changeable while downloads run. Every connection asks it how long to wait before it takes the bytes it just received; waiting delays the next read, so TCP's own flow control slows the server down. No data is dropped or buffered.

- **Pacing.** The limit books bytes on one imaginary pipe (`next` = when the pipe is free) and the caller waits until its bytes have "passed through". This gives an even rate with no bursts, whatever the number of connections, and it keeps the total right when many connections share one limit. Up to 50 ms of unused time is remembered so a connection that was idle for a moment can catch up a little.
- **Three levels.** A download carries a list of limits and goes at the slowest of them. The manager gives every download the global limit (saved in the `settings` table), its queue's limit (`queues.speed_limit`) and its own (`downloads.speed_limit`). Changing one takes effect at once, also for running downloads, and is saved.
- **Measured.** 10 MiB with 16 connections under 1 MiB/s took 10.4 s in a real run through the CLI (ideal 10.0 s); two 3 MiB downloads under one shared 1 MiB/s limit took 6.5 s (ideal 6.0 s). The limit is met from below, never exceeded.
- **The tuner** sees that more connections do not raise the speed under a limit and keeps few of them, which is the right outcome.
- **Not done:** speed profiles by time of day (part of the scheduler step).

## ADR-0014: Queue schedules

Status: accepted (17 October 2026)

A queue may have a schedule: a list of windows, each with days (`mon`..`sun`, empty for every day), an opening and a closing time, and an optional speed limit. The queue runs while any window is open and is stopped while none is. A window may cross midnight (`22:00` to `06:00`); such a window belongs to the day it opens, so "sat 22:00-06:00" is still open on Sunday morning. A queue without a schedule always runs. The schedule is stored as JSON in `queues.schedule_json`.

- **Checking.** The manager checks the schedules when it starts, whenever one is changed, and every 10 seconds. Time comes from a replaceable clock (`ManagerConfig::clock`), so tests move it by hand. Local time comes from `chrono`.
- **Closing.** Downloads running in a queue whose window closes are stopped the same way as on shutdown: progress is kept and they go back to `queued`, so they continue from where they were when the window opens again. Downloads waiting to retry go back to `queued` too, and nothing new starts in a closed queue.
- **Speed in a window.** While a window with a speed limit is open, its limit replaces the queue's own limit (the global and per-download limits still apply, and the slowest wins). Outside such windows the queue's own limit applies again.
- **Not done:** one-off "start at 02:00 tonight" jobs (a window does the same job every night), time zone or daylight saving changes while running (the clock is read each check, so the schedule follows local time), and per-queue schedules in the command line beyond the default queue.

## ADR-0015: After-download actions

Status: accepted (17 October 2026)

The manager can carry out one action when all downloads are done: `exit` (it only tells the application to close), `sleep`, `hibernate`, `shutdown`, or `run:<program> [arguments]` (started directly, with no shell). The rules are chosen so that nothing surprising can happen to the computer:

- **Always a countdown.** When the work is done, a countdown (60 s by default in the command line) is reported second by second as `ActionCountdown` events, and can be cancelled (`cancel_action`, Ctrl-C in the command line). A countdown of 0 is possible but has to be asked for.
- **Only for what finished after arming.** An action armed while nothing is downloading does nothing until a download finishes, so arming "shut down" on an idle list never shuts the computer down.
- **New work cancels the countdown, not the action.** Adding or resuming a download during the countdown stops it (`ActionCancelled { still_armed: true }`); the countdown starts again when the list is empty again. Downloads that are queued, waiting for a schedule or waiting to retry count as work.
- **Once, and not remembered.** After it ran, the action is disarmed. It is kept in memory only: a restart never inherits a pending shutdown.
- **Per operating system.** Windows: `shutdown /s /t 0`, `rundll32 powrprof.dll,SetSuspendState`, `shutdown /h`. macOS: System Events, `pmset sleepnow`. Linux: `systemctl poweroff | suspend | hibernate`. The system part sits behind the `ActionRunner` trait, so tests use a fake. Failures are reported as `ActionFailed`, never retried.
- **Not verified on every system:** the real shutdown, sleep and hibernate commands are only exercised by hand; automated tests use the fake runner. Sleep on Windows can instead hibernate if hibernation is on and sleep is off (a known quirk of `SetSuspendState`).

## 0016 – Categories and duplicate handling

Categories (name, extensions, folder) live in the database. A download added with sorting on has `auto_category` set; when it finishes, the task moves the file into its category's folder (absolute, or inside the download folder; never overwriting, numbering instead) and records the category. If the move fails the file stays put and the download still counts as complete. The newest category that lists an extension wins.

Duplicates are checked when adding: the same address already in the list (not counting failed or canceled ones), or a file with the name the address suggests already in the folder. The policy is ask (refuse, the default), rename, skip or overwrite. The name guessed from the address is only a guess; the server's own name is only known later, and the unique-name rule protects files in that case.

## 0017 – History and search

An FTS5 table (`downloads_fts`: name, address, category; one row per download, rowid = download id) is kept in step by triggers on `downloads`, so no code path can forget to index. Migration 004 backfills existing rows. What a person types is reduced to words of letters and digits, each matched as a prefix and all required, so `ubu 24` finds `ubuntu-24.04.iso` and no input can be an FTS syntax error. Filters (status, added-since, added-before, category name) combine with the text; results are newest first. Dates in the CLI are days in the local time zone, and `--until` includes the whole day.

The `log` table now gets a line when a download is added, completes, fails or is scheduled for a retry; `mohona log <id>` shows it. Renaming a category does not re-index old rows (it is a rare action and the name is only a search aid).

## 0018 – Network changes

The manager watches the local address the system would use to reach a public address (IPv4 and IPv6), found by "connecting" a UDP socket, which sends nothing. When it changes, or the network goes away, running downloads are stopped with their progress kept and put back in the queue, and nothing starts. When the network has been online and unchanged for `network_settle` (3 s), downloads start again from where they were. If there is no network when the manager starts, downloads wait the same way.

Why this and not a list of adapters: virtual adapters (Hyper-V, WSL, Docker) appear and disappear all the time and would stop downloads for nothing, while a Wi-Fi switch, a cable pulled, or a VPN going up or down changes the default route's address. What it cannot see: a change that leaves the address the same (a different router with the same IP range), which the normal automatic retries still handle. Checks run every 3 s (`network_check`). The probe is replaceable, so tests drive it.

## 0019 – Progress saves take the write lock first

The acceptance test "kill the app during 20 parallel downloads, start again" found that most downloads had no saved progress when killed, so they started over under new names and left old `.part` files behind. Cause: `save_progress` used a deferred transaction. In WAL mode a deferred transaction that starts by reading and then writes fails at once with "database is locked" if another connection committed in between, and the busy timeout does not apply to that case. With many downloads saving at the same time it happened constantly, and the failure was silent. The save now starts an immediate transaction, which waits for the lock (up to the 5 s busy timeout). A store test with 8 threads saving progress at once guards it, and the kill-and-restart test runs the real program (`crates/cli/tests/acceptance.rs`).

Acceptance coverage for Phase 2: queue limit never exceeded (manager test), speed limit holds across 16 connections (core test), schedule opens/closes and survives restart (manager tests), network switch pauses and resumes (manager tests, and by hand on Windows), kill during 20 parallel downloads (CLI test).

## 0020 – Request context (Phase 3, steps 1 and 2)

A download can carry a `RequestContext`: cookies, referrer, user agent and any other headers. It is sent with every request of that download (probe, plain GET, each ranged GET), is saved as JSON in the `headers_json` column so retries and restarts send it again, and is set with `-H`, `--cookie`, `--referer` and `--user-agent` on `get` and `queue add`. The browser extension will fill it in Phase 5. Headers that would break the transfer (`Range`, `Host`, `Accept-Encoding`, `Connection`, ...) are refused up front, and the `Debug` output never shows cookie or header values.

Known limits, on purpose for now:
- Cookies and header values are saved in the database in plain text. The plan's security list wants secrets in the system keychain; that comes with the desktop app and the credential work in this phase.
- On a redirect to another host, the HTTP library drops `Cookie` and `Authorization`, as browsers do, but custom headers and `Referer` are still forwarded. Do not put a secret in a custom header for a link that redirects to a host you do not trust.
- The plan's "one transport trait" step is postponed until the second transport (FTP) exists. A trait designed from one implementation tends to be wrong for the second; `HttpClient` already has the shape (probe, get, get_range) the trait will need.

## 0021 – Basic and Digest authentication (Phase 3, step 3)

Credentials (`user:password`) are part of the request context. They are not sent first: a request goes out plain, and only when the server answers `401` with a challenge is the answer sent, Digest preferred (SHA-256 before MD5, `qop=auth` or the old form, `-sess` variants), else Basic. What worked is remembered for the rest of that download, so its many connections do not each start with a refused request. Digest counts its `nc` per nonce; when connections race and the server refuses an out-of-order count, or a nonce expires, the client answers the new challenge and carries on (tested with a strict server that rejects replays and expires nonces).

Rules that keep the password safe:
- It is only sent to the origin (scheme, host, port) the request was addressed to. A challenge from a server reached through a redirect is not answered, and the HTTP library drops `Authorization` on cross-host redirects itself.
- A `user:password@` in an address is taken out of it before anything is stored, so the download list and logs never hold it.
- `Debug` output hides the password. On the command line: `-u user:password`, or `-u user` with `MOHONA_PASSWORD` set (a password typed on the command line stays in shell history).

Limits: the password is saved in the database in plain text with the download's request details (same limit as ADR-0020; keychain comes later). Basic over plain `http://` sends the password readable on the network; the server decides which scheme it asks for, so use `https://` where it matters. `auth-int`, `userhash`, NTLM and Negotiate are not supported (NTLM and Negotiate are last in this phase and optional).

A `401` now becomes its own error (`NetError::Unauthorized`) that names the kinds of login the server asked for and says what to do: give `-u`, the user name or password was refused, the server wants something Mohona cannot do (Bearer, NTLM, Negotiate), or it did not say how to log in (a web login page: use `--cookie`).

## ADR-0022: Proxies are part of the request context

**Decision.** A download may carry a proxy choice (`ProxyChoice::Direct` or a proxy address) inside its request context, so it is saved with the download and survives restarts. Supported: `http://`, `https://`, `socks5://` and `socks5h://` (names resolved by the proxy). An address with no scheme is read as `http://`. A login goes in the address (`user:pass@`) and is never printed. With no choice, the system proxy is used, as before.

**Why.** Proxies are per-network and sometimes per-download; the context already travels with every request and is persisted. A different proxy needs a different connection pool, so a download with its own proxy gets its own client; others keep sharing connections.

**Errors.** A proxy that asks for a login we did not give is reported as such (407), not as a mystery failure.

**Tests.** Test proxies count what they relay, so tests prove the traffic really went through them, including login, wrong login, and socks5 (address) versus socks5h (name).

## ADR-0023: Certificates are checked strictly, with two deliberate exceptions

**Decision.** HTTPS certificates are always checked against the computer's trusted authorities. A download may add its own authority (`--ca-file`, PEM) or skip the check (`--insecure`). Both are stored in the download's request context, so they apply to that download only and survive restarts. There is no global "ignore certificates" setting and `--insecure` is never implied.

**Why.** Skipping the check silently makes every download readable and changeable by anyone on the path. Private servers are real, so the safer way out (trust one extra authority) is offered first and named in the error message.

**Errors.** A refused certificate is its own error, not retried, explaining the cause (unknown authority, expired, not yet valid, wrong name) and the two ways out.

**Tests.** The test server can speak HTTPS with a generated authority and certificate, expired or not, so each case is exercised for real.

## ADR-0024: Checksums are checked after the download, and a mismatch keeps the file

**Decision.** A finished file may be checked against a checksum from, in order of priority: the person (`--checksum`), a checksum file next to the download (`--find-checksum`, opt-in), or a hash the server announced for the whole file (`Content-MD5`, `Digest`, `Repr-Digest`; the strongest wins). The check hashes the finished file once (MD5, SHA-1, SHA-256, SHA-512). A mismatch is an error that is not retried, and the file stays where it is, under its normal name.

**Why.**
- Hashing while downloading does not work with several connections writing out of order, and resumed downloads would need the earlier part re-read anyway. One pass over the finished file is simple and correct.
- Looking for neighbor files costs requests (and some servers answer 200 with a web page for anything), so it is opt-in. Text that does not parse as a checksum, or a list that does not name the file, counts as "no checksum found", never as a failure.
- Hashes from `206` answers describe a piece, not the file, so only whole-file answers are trusted.
- Deleting a file after hours of download because of a wrong checksum (which may be a typo) is worse than keeping it; the message says where it is.
- A repeat would give the same bytes, so a mismatch is not retried.

**Stored** with the download in the request context, so it survives restarts.

## ADR-0025: Mirrors are extra sources for the same file, checked by size

**Decision.** A download may list other addresses of the same file (`--mirror`, stored in the request context). Each mirror is probed; it is used only if its size equals the main address's size and it accepts byte ranges. Segments are handed out across the usable addresses, a failing address is given up after repeated failures with no data, and its segments are taken by the others. If the main address cannot be reached at the start, the first mirror that answers is the main one. If the file cannot be fetched in pieces, mirrors are ignored.

**Why.**
- Spreading segments over servers adds their speeds and survives one of them failing, using the segment machinery that already exists.
- The size is the only thing known cheaply about a mirror. It catches stale versions, not wrong content: the docs say so and point to `--checksum`, which verifies the whole file at the end.
- Mirrors are never trusted with more than the main address would be: same cookies, login, proxy and certificate settings, and each address has its own `If-Range` tag so a changed file is noticed.
- Mirrors are never fatal: every skip or failure is a note, and only "no address works" is an error.

## ADR-0026: FTP is handled inside the same client, chosen by the address

**Decision.** `HttpClient` serves `ftp://` addresses too: `probe`, `get` and `get_range` look at the scheme, and the body it returns reads from either kind of connection. FTP is written by hand (about 300 lines) over tokio: passive mode (`EPSV`, then `PASV`), binary type, `SIZE`, `MDTM`, `REST`, `RETR`. Each request opens its own control connection and (for data) its own data connection.

**Why.**
- The download engine already needs exactly three things from a transport (probe, read from here, read this range). Routing by scheme keeps the engine, mirrors, checksums, queue, retries and the tuner unchanged and working for FTP at once. A separate transport trait would have touched every call site for no gain yet. When SFTP and WebDAV arrive, the same switch is extended; if it grows unwieldy, the trait comes then, with three real implementations to design it from.
- One connection per piece mirrors HTTP ranges, so segments, resume, takeover of slow pieces and "fewer connections when the server pushes back" all work as they are. A `421` is treated like HTTP 429.
- The address in a `PASV` answer is ignored and the control connection's address is used (the usual fix for servers behind NAT).
- `MDTM` serves as the validator (like an ETag) so a changed file is noticed on resume. A server without `REST` or `SIZE` gets a single plain download.
- A part ends by hanging up, not by sending `ABOR`: simpler, and what download tools do.
- Credentials: the address's login or `-u`; otherwise `anonymous`.
- Hand-written rather than a library because the surface is small, the tests need a matching hand-written server anyway (ADR-0004), and it keeps the dependency list short.

**Not yet:** FTPS (next, with the TLS settings of ADR-0023), FTP through proxies, folder listings, active mode.

## ADR-0027: FTPS is chosen by the scheme, always encrypts the data too, and uses the HTTPS certificate rules

**Decision.** `ftps://` is implicit TLS (default port 990); `ftpes://` is explicit TLS (`AUTH TLS` on port 21). `ftp://` never upgrades by itself. After login, `PBSZ 0` and `PROT P` make data connections encrypted too. Certificates are checked with the same rules as HTTPS (system trust store through the platform verifier, plus `--ca-file`, or `--insecure`), and the same error message and meaning (ADR-0023).

**Why.**
- The scheme says what the person expects. Silently upgrading `ftp://` would make failures confusing against servers with broken TLS, and silently *not* upgrading `ftpes://` would be a security downgrade, so `ftpes://` fails if the server refuses `AUTH TLS` and no login is sent.
- Encrypting only the commands would protect the password but not the files, which is rarely what people mean by FTPS.
- One TLS configuration is made per client and shared by the control and data connections, so a data connection can resume the control connection's TLS session, which servers like vsftpd require by default.
- Many FTP servers close the encrypted data connection without the TLS closing message. That is accepted, because the file's size and the server's final `226` reply tell whether everything arrived.
- The platform verifier is used (as `reqwest` does) so Windows' own trust store and company inspection authorities apply the same way for HTTPS and FTPS.

## ADR-0028: SFTP uses russh, checks the host key first, and keeps key passphrases out of the database

**Decision.** SFTP is served from the same client and chosen by the `sftp://` scheme (ADR-0026), built on the `russh` and `russh-sftp` crates (pure Rust, async, no C library to build on Windows). Every request opens its own SSH connection, logs in, and reads with the file's offset, so pieces, resume and slow-piece takeover work as for HTTP and FTP.

**Host keys.** The server's key is judged before any credential is offered: `--host-key` (an exact fingerprint) wins; otherwise the OpenSSH `known_hosts` file (the person's own, or `--known-hosts`) decides. An unknown server is refused with its fingerprint and the two ways to trust it. `--trust-new-host` records a new server (like OpenSSH's `accept-new`). A server whose key *changed* is always refused, even with `--trust-new-host`, because that is exactly what an interception looks like. `--insecure` (the same flag as for certificates) turns the check off.

**Login.** A user name is required (no "anonymous" for SSH). `--key` offers that key, then the password if there is one. Without `--key`, a password is used if given; otherwise the default keys in `~/.ssh` without passphrase are tried. No SSH agent and no `~/.ssh/config` yet.

**Secrets.** A key's passphrase comes from `MOHONA_KEY_PASSPHRASE` and is never written to the database. Passwords follow the existing rule for logins (saved with the download until the keychain work).

**Errors.** Host key problems, login failures and missing or forbidden files are final (not retried); connection and protocol hiccups are retried like HTTP. Messages say what to do.

**Why not `ssh2`/libssh2:** blocking calls and a C dependency (OpenSSL or libssh2 builds on Windows are a recurring source of trouble); `russh` also gave a test server in the same language for ADR-0004-style tests.

## ADR-0029: WebDAV addresses are HTTP(S) addresses

**Decision.** `webdav://` and `dav://` are read as `http://`; `webdavs://` and `davs://` as `https://`, in one place where addresses are parsed. There is no separate WebDAV transport.

**Why.** Reading a file from a WebDAV server *is* an HTTP `GET`, with the same ranges, validators and Basic or Digest logins that already work. What the schemes add is only that people (and file managers such as GNOME's `dav://`) write them, so they should be accepted and mean what is expected, including the certificate rules for `webdavs://`. The address of the finished request after redirects is an ordinary http(s) address.

**Not built:** `PROPFIND` to list folders or to learn a file's size on servers that refuse `HEAD`. `HEAD` plus a one-byte range request has worked on every server tried, so this waits for a server that needs it, or for folder downloads.

## ADR-0030: Media manifests are parsed in their own network-free crate

**Decision.** `mohona-media` turns the text of a manifest into one common model (`MediaVariant`, `Track`, `Segment`, `SegmentList`) and does no network work. HLS comes first (`hls::parse`); DASH will produce the same model. Fetching playlists and segments is done by the callers with the normal transports, so proxies, logins, cookies and certificate rules apply to media exactly as to files.

**Why.**
- Parsing is pure, so it is tested with plain text and covers odd playlists without any server.
- One model means the CLI, manager and UI never care whether a stream was HLS or DASH.
- Offsets of byte-range pieces, init segments, keys and IVs are resolved *while parsing*, so every `Segment` can be downloaded on its own (needed for parallel fetching and for resuming by segment index).

**DRM.** `SAMPLE-AES`, and any key format other than `identity` (FairPlay, Widevine and the like), end in `MediaError::Protected` with a message saying DRM is not supported, before anything is downloaded. Only plain AES-128 is accepted.

**Not yet handled:** `EXT-X-SESSION-KEY`, I-frame playlists and closed captions inside the video are ignored on purpose; live playlist reloading belongs to the segment downloader.

## ADR-0031: DASH manifests become a flat list of renditions with every segment listed

**Decision.** `dash::parse` returns `Vec<Rendition>`: one entry per (kind, representation id), with all its segments already resolved to addresses, byte ranges and init segments, periods joined (the first segment of a later period is marked `discontinuity`). It understands `SegmentTemplate` (by `$Number$`, by `$Time$`, with `SegmentTimeline` including `r="-1"`), `SegmentList`, `SegmentBase` and nested `BaseURL`s.

**Why.** A DASH manifest is a tree whose rules (what a representation inherits from its set, how templates expand) are exactly the part that is easy to get wrong in the downloader. Doing all of it while parsing gives the segment downloader the same simple input as for HLS: a list of independent pieces. Choosing quality, audio language and subtitles then works on `Rendition` for DASH and on `MediaVariant` plus `Track` for HLS, with one picker later.

**On-demand files** (`SegmentBase`, a single `BaseURL`) are one whole-file segment, since the index and init parts are already inside the file.

**Refused or not yet handled.** Any `ContentProtection` is refused as DRM (`MediaError::Protected`), because even common-encryption streams cannot be played without a licence. Live (`type="dynamic"`) manifests are refused with a clear message until the live-recording step. Template durations need a known period length (`mediaPresentationDuration` or `Period@duration`).

## ADR-0032: Stream segments are saved one by one in a `.parts` folder and joined at the end

**Decision.** `download_stream` fetches segments (8 at a time by default), retries each on its own with the usual retry rules, decrypts AES-128 pieces, and saves each finished piece as its own file in `<name>.parts/`. A piece is written under a temporary name and renamed, so it exists completely or not at all. When all pieces are there they are joined in order (an init segment goes in front whenever it changes) into `<name>.tmp`, which is renamed to the result, and the folder is removed.

**Resume.** Resuming is "which piece files exist": nothing else needs to be saved, and a kill at any moment is safe. A `plan` file in the folder holds a fingerprint of the segment list (count, first and last address and range); a different playlist wipes the folder, so pieces of another stream are never mixed in.

**Why not write straight into one file.** Segments finish out of order and differ in size, so a single file would need a separate index to resume, and decryption works on whole pieces anyway. The cost is that the result is copied once at the end and needs room for the pieces plus the file; segments are small, so it is quick, and the simpler failure behaviour is worth it.

**Keys.** A key is fetched once per address (guarded by a lock so parallel segments do not all fetch it) through the same client, so cookies, logins and proxies apply. A key that is not 16 bytes is almost always a login page; the message says so.

**Output.** The result is the pieces joined as they are: right for `.ts` and for fragmented MP4 of one track. Separate video and audio renditions are joined into one file by the muxing step (ffmpeg, next).

## ADR-0033: `mohona stream` is its own command, and quality is picked by simple rules

**Decision.** Streams have their own command (`mohona stream`) instead of `get` guessing from the address, because stream addresses often have no `.m3u8` or `.mpd` ending and because the options differ (quality, audio, listing). The command reads the document and tells HLS from DASH by its first characters.

**Quality rules** (in `mohona-media::quality`): `best` is the highest bit rate; `worst` the lowest that still has video; a number means "the best one not taller than this", and if all are taller the smallest, so a request is never refused for lack of an exact match; `audio` is the best sound-only stream. A stream that has no video at all gets its best audio for any quality. Audio language matching accepts the language code (`en`, also `en-US`) or the track name, in any letter case.

**Why.** People think in "720p", not in bit rates, and "not taller than" is what players do. Never failing for a missing exact height keeps scripts working when a site changes its ladder.

**For now** video and audio that are separate end up as two files; the ffmpeg step joins them. Remembering the choice per site comes with the settings work.

## ADR-0034: Joining video and audio runs ffmpeg as a separate program, copying only

**Decision.** When a stream has separate video and audio, `mohona stream` downloads both and then runs `ffmpeg -c copy` (video stream 0, audio stream 0) to make one MP4 (or MKV with `--container mkv`). MP4 gets `+faststart`. The result is written under a temporary name and renamed, and the two downloaded files are deleted only after ffmpeg succeeded (and never if one of them has the result's own name).

**Finding ffmpeg.** `--ffmpeg` (a file or a folder), else the `MOHONA_FFMPEG` variable, else next to the `mohona` program, else the `PATH`. An explicit choice that does not exist is an error for that choice, not a reason to search elsewhere.

**Failure is not fatal.** No ffmpeg, or ffmpeg refusing the files, never fails the command: the downloads are good, so they are kept, with a message that says what to do. `--no-join` skips the step on purpose.

**Why copy only.** Re-encoding is slow, loses quality and needs far more of ffmpeg than most builds guarantee; copying is what people expect from a download manager. Codec/container mismatches are reported with ffmpeg's own words and fixed by choosing MKV.

**Licence.** ffmpeg is never linked or bundled by this step, so Mohona's licence is unaffected (see the Phase 8 packaging note for how a bundled build will treat it).

## ADR-0035: Subtitles are converted to SRT; embedding is optional and uses ffmpeg

**Decision.** `--subs` downloads the chosen subtitle tracks with the same segment downloader as video, converts WebVTT or TTML to SRT in Mohona (`mohona_media::subtitles`), and saves `NAME.LANG.srt`. `--embed-subs` then adds them to the finished video as tracks with ffmpeg (`mov_text` for MP4, `srt` for MKV, language tags widened to ISO 639-2), keeping the `.srt` files.

**Why SRT, and why in Rust.** SRT is the one format every player, editor and ffmpeg takes. Converting by hand keeps the common case (no embedding) free of ffmpeg, and lets the converter deal with how streams really look: segmented WebVTT is joined as it comes, a new `WEBVTT` header starts a new block even when the previous file ended without a blank line (found by a test), cues crossing a segment cut appear in both files and are listed once, tags other than `<i>`, `<b>`, `<u>` are removed, and HTML entities are decoded.

**Limits (stated, not hidden).** `X-TIMESTAMP-MAP` offsets are ignored, so a stream whose video does not start at 0 may need a shift (listed in the manual tests). TTML supports clock times, offsets (`s`, `ms`, `h`, `m`, `f`, `t`) and `<br/>`, not styling or regions. Subtitles inside MP4 segments are refused with a clear message.

**Failure is not fatal.** A subtitle track that cannot be fetched or converted is reported and skipped; the video is never touched unless embedding succeeded, and embedding writes a temporary file first.

## ADR-0036: Live HLS is recorded straight into one file; a finished file is never replaced

**Decision.** When a playlist has no end (`#EXT-X-ENDLIST` missing), `mohona stream` records it: it reloads the playlist (about every half segment length, 0.5 to 5 s), fetches each new segment once, a few at a time, and appends them in order to one file. It stops when the stream ends, at `--record-for` (seconds of *recorded video*, like `30m`), at Ctrl-C, or when the playlist disappears (usually a stream that ended without saying so; what was recorded is kept). Separate audio is recorded alongside and joined afterwards like for any stream.

**Why not the `.parts` folder.** A recording has no known size and nothing to resume (the segments that slipped out of the window are gone), but it must survive being stopped at any moment. Appending finished segments in order gives a file that plays at every moment, and stopping or crashing loses at most the segment in flight. The cost is that a recording cannot be resumed; a second run records a new file.

**Gaps are reported, not hidden.** When the server's window moves faster than we fetch, the missing segment count is shown as a warning.

**Naming.** Because a recording starts a new file each time, and so that finding the stream already ended must not destroy an earlier recording, `mohona stream` now picks a free name (`show (1).ts`) for every stream download, like `get` does. This replaces the earlier behaviour of silently overwriting a finished file of the same name.

**Not supported yet:** live DASH, subtitles for live streams (`--subs` is refused with a message), and live streams that only partly end.

## ADR-0037: Preview copies the complete start of a partial file; convert is a handful of fixed ffmpeg presets

**Preview.** A `.part` file is preallocated: the end is zeros and the middle may be missing while several connections work. Players cope badly with that, and the `.part` name hides the file type. So `mohona preview` asks the progress ledger how many bytes from the start are complete with nothing missing before them (`Ledger::contiguous_bytes`), copies exactly that part into a temporary file with the final name, and opens it with the system's own opener (`start`, `open`, `xdg-open`). The download is never touched. Copying costs time for very large partial files, accepted for being correct on every platform and player (a hard link would show the zero-filled tail). Downloads kept in the queue database are not found yet, because their progress is not in a sidecar file.

**Convert.** Four presets (`mp3`, `m4a`, `mp4`, `mp4-small`) with fixed, boring ffmpeg settings (libmp3lame quality 2; AAC 192k; H.264 CRF 23 and 28, `yuv420p`, `+faststart`, small one scaled to at most 720 lines). Presets, not free-form options, because the point is "make it play everywhere" without learning ffmpeg; anyone who needs more runs ffmpeg directly. The result is written under a temporary name and renamed, and never replaces an existing file.

## ADR-0038: The browser host hands downloads to a background engine (`mohona daemon`)

**Decision.** The native messaging host (`mohona-host`) never touches the database. It talks to `mohona daemon`, the background engine, through its local API (JSON lines on `127.0.0.1`, with a secret token in a file only the person's account can read). Messages from the extension: `hello` (protocol, versions, whether the engine answers), `add` (url, referrer, cookies, userAgent, headers) and `startEngine` (starts `mohona daemon` detached and waits up to 8 s for an answer).

**Why not write to the database from the host.** A first version did that and used a heartbeat so the host could tell whether anything was running the queue. It meant two programs writing to one database, a polling delay before downloads started, and a heartbeat that stale after a crash. One engine owning the database and the downloads is simpler and correct: `add` takes effect immediately, duplicates are decided by the same code as the CLI, and the engine answers "who is there" honestly.

**Safety at the edge.** The host is the one place where web-controlled text arrives. It accepts only http, https, ftp, ftps and sftp addresses, refuses control characters in every field, caps messages at 1 MiB, and forwards only cookies, referrer, user agent and plain headers (never proxy, certificate or key settings; the daemon enforces this again). Duplicates are sent with policy `skip`, so the extension is told about them instead of silently creating a second copy. Standard output carries only protocol frames.

**Info file.** Default: `%LOCALAPPDATA%\Mohona\daemon.json` on Windows, `$XDG_DATA_HOME` or `~/.local/share/Mohona/daemon.json` elsewhere; `--info` overrides it for `mohona daemon` and `mohona-host`. It is removed when the daemon exits normally; a leftover file from a crash is detected (nobody answers) and replaced.

## ADR-0039: Host registration is per user, done by the host program itself

**Decision.** `mohona-host install [--browser chrome|chromium|edge|firefox|all] [--extension-id ID]`, `uninstall` and `status` register the host for the current user only; no administrator rights. The host name is `org.mohona.host`. The manifest names the host program by absolute path (the running executable) and the one extension allowed to use it: `allowed_origins: ["chrome-extension://<id>/"]` for Chrome, Chromium and Edge, `allowed_extensions: ["mohona@mohona.local"]` for Firefox. Never a wildcard: any other extension (or web page through one) is refused by the browser itself.

**Where.** Windows: the manifest is kept in `%LOCALAPPDATA%\Mohona\native-messaging\<browser>.json` and a key `HKCU\Software\<vendor>\NativeMessagingHosts\org.mohona.host` points at it (set with the built-in `reg` command, no extra dependency). macOS and Linux: the manifest goes into the browser's own `NativeMessagingHosts` folder (`~/.config/google-chrome/...`, `~/.mozilla/native-messaging-hosts`, `~/Library/Application Support/...`).

**Chromium ids.** An extension id comes from its key; the extension will pin its key so the id is the same on every machine, and the installer takes it with `--extension-id` (checked: 32 letters a to p) or `MOHONA_EXTENSION_ID`. Without it, Chromium browsers are skipped with a message rather than registered for everyone.

**One database.** A browser starts the host with no useful working folder, so the host and `mohona daemon` share a default database, `mohona.db` in the Mohona data folder (`%LOCALAPPDATA%\Mohona` on Windows). `mohona daemon` uses it unless `--db` is given; `mohona-host` uses `--db`, `MOHONA_DB`, or it.

## ADR-0040: The extension is TypeScript built by Vite into two plain folders

**Decision.** `extension/` holds a Manifest V3 extension written in TypeScript. `npm run build` produces `dist/chromium` (Chrome, Edge, Chromium) and `dist/firefox`, each with `manifest.json`, `background.js`, `popup.html` and `popup.js`. The two scripts are built as self-contained classic scripts (IIFE) so every browser loads them the same way; the only differences between the browsers are in the manifest (a service worker for Chromium, `scripts` plus a Gecko id for Firefox, Firefox 128 or newer).

**Permissions.** Only `nativeMessaging` for now. No host permissions, no content scripts. Later steps add what they need one at a time (`downloads` for interception, `contextMenus`, `webRequest`...) so that each addition is visible in review and in the store listing.

**Stable id.** The Chromium manifest carries a public `key`, so an unpacked or sideloaded extension always has the id `iflnmaneimjgoibmpcmlkllhjccebbhj`. `mohona-host install` registers that id by default (`register::CHROMIUM_ID`); a test in the extension checks that the id really is derived from the key and appears in the Rust source. The private key was thrown away: it is only needed to sign a `.crx` yourself, and the Chrome Web Store assigns its own id when the extension is published (then `--extension-id` is used, or the default changes).

**Talking to the host.** One long-lived `connectNative` port per browser session, questions carry a `requestId`, each has a timeout, and a lost port fails the questions waiting on it and reconnects on the next one. `addDownload` starts the engine once when the host says `engine_not_running`, then tries again. This logic does not touch browser APIs (the port is passed in), so it is unit-tested with a fake host; a Playwright script (`extension/e2e/chromium.cjs`) checks the whole chain in a real Chromium.

## ADR-0041: Download interception pauses, hands over, then cancels, and fails open

**Decision.** The background script listens to `downloads.onCreated`. For each browser download that qualifies it (1) pauses the browser's download, (2) reads the site's cookies and sends address (after redirects), referrer, cookies and user agent to Mohona through the host, and (3) only when Mohona answers `added` cancels and erases the browser's copy. In every other case the browser's download is resumed untouched: engine missing and cannot start, host not installed, timeout, refusal, a protocol error, or a duplicate. The rule is that installing Mohona can make a download faster but never lose one. A duplicate (the address is already in Mohona's list) is also left to the browser, because silently dropping the click could lose a download whose first attempt failed.

**What is left to the browser.** Private windows (nothing from a private window is handed to another program), anything that is not http, https or ftp (`blob:`, `data:`, `file:` and so on), sites on the exclusion list, and, when a limit is set, downloads known to be smaller than it. All of this is one pure function (`skipReason`) with tests.

**Permissions.** `downloads` (the event, pause, cancel, erase), `cookies` plus `host_permissions: <all_urls>` (a download can come from any site and needs that site's cookies to work, the main reason IDM-style managers exist), `storage` (settings). No content scripts: nothing runs inside web pages. Reading cookies is best-effort: when it fails the download is still handed over without them. Firefox treats host permissions as grantable by the user; where they are not granted, downloads that need cookies fail in Mohona and can be retried in the browser.

**Known limit.** The browser starts a download before the extension hears of it, so a very small file may finish before the pause; then the file is in the browser and Mohona also gets it (a duplicate, harmless). A later step will use `onDeterminingFilename` where it helps.

**Settings** (`storage.sync`, so they follow the person between browsers): `intercept` (on by default; the popup has the switch), `minSizeBytes`, `excludedHosts`. Stored values are re-validated on every read.

## ADR-0042: A right-click entry for links and media

**Decision.** One context-menu entry, "Download with Mohona", on links, images, video and audio. The address is the link when the click was on one (even around an image), otherwise the media's own source. Only http, https and ftp addresses are sent; `blob:`, `data:`, `javascript:` and `mailto:` entries do nothing. The referrer is the frame the link was in (or the page), and the site's cookies are sent as for intercepted downloads.

**Feedback.** The menu has no window to show a result in, so the toolbar button shows a green check for a few seconds when Mohona took the address and a red "!" when it did not; the popup then says why ("Not sent: ..."). Unlike interception, a failure here is not silent and not retried by the browser: the person asked Mohona explicitly, so a duplicate ("Mohona already has it") is reported as a failure rather than hidden.

**Not done.** "Download all links on this page" and "download selected links" are a separate, larger feature (a list to pick from) and are left for the settings/options step.

**Permission.** `contextMenus`. The menu is created in `runtime.onInstalled` (the browser keeps it between worker restarts).

## ADR-0043: Media detection looks at responses; streams run as jobs of the engine

**Detection.** The extension watches response headers only (`webRequest.onHeadersReceived`, not blocking, no content script, the page is never read). A response is media when it is an HLS playlist (`.m3u8` or an mpegurl type), a DASH manifest (`.mpd` or `application/dash+xml`), or a video or audio file (type or extension). Pieces of streams (`.ts`, `.m4s`, `.aac`, subtitles, keys) are ignored, since the playlist is what to download, and so are files known to be under 200 KB (ads, previews). The list per tab is kept in session storage (the worker can be stopped at any time), cleared on every new top-level page and when the tab closes, newest first, at most 30. The toolbar button shows the count for the tab. Names come from the address, with the page title as the nicer alternative later.

**Downloading.** A detected file goes through the normal `add` (multi-connection download with the page as referrer and its cookies). A playlist or manifest goes to the new host message `stream`, which asks the engine to run `mohona stream` for it. The stream downloader lives in the command line program, so the engine runs that program as a job instead of duplicating it in a second place; the job list (`running`, `finished`, `failed` with the program's last message) is in the engine and visible through the `jobs` call. The person's choices (quality, audio language, subtitles) are not asked yet: the program's defaults (best quality, default audio, joined with ffmpeg when installed) apply, and the popup says where the file will be saved.

**Safety.** Everything a web page influences reaches the program as a separate argument in the form `--option=value`, with the address after `--`, so it cannot become another option; the name must be a plain file name (no folder, no `C:`, not starting with a dot); control characters are refused; only http and https are accepted. The session cookie is passed in the environment variable `MOHONA_COOKIE` and never on the command line, where other programs could read it from the process list. Jobs are children of the engine and end with it.

**Limits.** Live streams record until they end or are stopped from outside (`--record-for` is not offered yet); protected (DRM) streams fail with the program's message and show as `failed`; there is no progress yet beyond the job state.

## ADR-0044: Files the browser holds (`blob:` and `data:`) are read by the extension and sent in pieces

**Decision.** The right-click entry also works on `data:` and `blob:` addresses. A `data:` address is decoded by the browser's own `fetch`. A `blob:` address only exists in the page that made it, so the extension runs one small function inside that page (`chrome.scripting.executeScript`, only on a click, only in the clicked frame) that returns one slice at a time as base64; nothing is left behind in the page and the page is not otherwise touched. The pieces (512 KiB, base64 about 683 KB, under the 1 MiB limits of the browser-host channel and the engine's request line) go through new host messages `saveStart`, `savePiece`, `saveEnd` and `saveAbort`, which the engine's `receive` module assembles into a temporary file next to the destination and renames when complete.

**What the engine decides.** The kind of file comes from its first bytes (PNG, JPEG, GIF, WebP, PDF, ZIP, MP4, M4A, WebM, MP3, Ogg, FLAC, WAV, BMP, ICO, SVG, gzip, 7z, RAR), then from the media type when the bytes say nothing (text, JSON, CSV, HTML and so on). The extension of the suggested name is corrected when it is a known media or document type that does not match (`photo.jpg` that is really a PNG becomes `photo.png`; `report.docx` stays, since it is a zip), and added when missing. Names are cleaned of folders, control characters and anything Windows refuses (`CON`, trailing dots, `:` and so on), and a taken name becomes `name (1).ext`: nothing is ever replaced. At most four files are received at once, the limit is 1 GiB, pieces must come in order, an error removes what was received, and a transfer that stays idle for ten minutes is removed.

**Why through the engine at all.** There is no network transfer to speed up. The reasons are one folder and one naming rule for everything, correct file types (browsers often save pasted images as `download` with no extension), and the same place for later features (history, categories). Interception still leaves `blob:` and `data:` downloads to the browser, so the browser's own Save keeps working exactly as before.

**Limits.** Blob URLs that the page has already released cannot be read ("the page no longer has this file"); a very large blob is read once per piece, which costs time but keeps memory low. These saves are not in the download list yet: they are plain files in the download folder.

## ADR-0045: Expired links are replaced from the page they came from, and checked before use

**The problem.** Many sites sign their download links and sign them again every time the page loads, so a link that fails with 403 or 410 after a pause, a restart or a long queue is usually fine again with a fresh one from the same page.

**Engine.** `Manager::refresh_link(id, url, update)` gives a failed, paused or cancelled download a new address and new request details (cookies, referrer, user agent; what is left out stays) and queues it. Whatever was downloaded stays, and the download resumes from there. Before anything changes, the new link is probed with the download's own request details, and it is refused (nothing is changed) when: it does not work; the old file's size is known and the new link has a different or unknown size; the `ETag` or `Last-Modified` changed (a different version of the file); or the file name differs and nothing else (ETag, date) vouches for it. When nothing was learned about the file yet (the very first request was refused), only the link itself is checked. A running or finished download is never touched. This is the same rule the resume logic already uses (same size, same ETag), so a refreshed link can never glue two different files together.

**API.** The daemon's `list` marks a failed download `expired` when its error is a 403 or 410 answer, and gives its source `page` (the referrer saved when it was added); `refresh` continues it. The host has `list` and `refresh` messages (a refused link comes back with code `refused` and the reason).

**Extension.** For an expired download with a known page, the extension opens that page in a background tab, waits for it to load and settle, reads every address on it (links, video and audio sources, plus the media the sniffer saw), closes the tab, and picks the address that looks like the old one: first the same site and path with a different query (the usual new signature), then the same file name on another host. It never picks the link that already failed. The engine then decides whether it is really the same file. This runs automatically once a minute while the engine is running (it never starts the engine for this), one download per round, at most two tries per download per browser session, and can be switched off ("Refresh expired links automatically"); the popup also lists expired downloads with a "Refresh link" button.

**Limits.** Sites that need a click or a captcha to produce a link, links that change the file name and size, and pages that need a login the browser does not have, are not solved; the popup says that no matching link was found, and the download stays in the list for the person to retry. A page that starts a download by itself when opened would be taken over like any download and shows up as a new entry.

## ADR-0046: A small download button on videos, from a page script that sends nothing until clicked

**The problem.** The popup lists what a page played, but most people expect the button next to the video, as other download managers have it.

**Decision.** The extension now has one content script (`content.js`, web pages only: `http://*/*` and `https://*/*`, all frames, after the page is ready). It looks for `<video>` elements at least 240×135 pixels and puts a small "⬇ Mohona" button in the top right corner of each, inside a shadow root so the page's styles and scripts cannot reach it, and it keeps the button in place when the page scrolls or the player changes size. The script reads nothing from the page except the player's size, position and its `http(s)` address. Nothing is sent anywhere until the button is clicked; then it asks the background script for the media the tab has shown (the sniffer's list) plus the video's own address, and the person picks one. A `blob:` address is never offered (it is no address); the sniffer list is what covers those players. Downloads use the same `downloadMedia` path as the popup.

**Switches.** "Show a download button on videos" in the popup (`videoButton`, on by default) and "Hide on this site" in the button's own panel (`buttonHiddenHosts`, a site and its subdomains). Both live in the synced extension settings, and open pages react to a change at once.

**Cost.** This is the first script that runs inside every web page, so it is kept small, does its work after a short delay when the page changes, and does nothing at all while the button is switched off. The store text (step 10) must say so.

## ADR-0047: Settings the app and the extension share live in the engine, as one text, and the newer copy wins

**The problem.** The extension has settings (take over downloads, minimum size, excluded sites, the video button and its hidden sites, automatic link refresh). The desktop app will show the same ones, and `chrome.storage.sync` is per browser: Chrome, Edge and Firefox on one computer do not share it.

**Decision.** The engine keeps one value, `extension.settings` (JSON text, at most 64 KiB), in its `settings` table. It never reads it. Clients may only use keys that start with `extension.` (the engine's own settings are out of reach). The host passes it through as `getSettings` and `setSettings`. The extension compares its copy with the engine's when it starts, every minute (with the expired-link round) and after each change a person makes: the copy with the larger `updated` time (set only when a person saves a setting, never when a copy is adopted) is taken by the other side; equal copies change nothing. Unreadable or invalid engine text is replaced by the extension's copy, and every value is checked (`sanitize`) before use, so a bad copy can never switch on anything unexpected. Nothing happens when the engine is not running; the next round catches up. The popup edits the minimum size and the excluded sites, and the other checkboxes.

**Limits.** The newer whole copy wins; two changes made on two browsers between rounds are not merged field by field (the later one wins). The file-type list is not a setting yet: what to take over is decided by the browser's own download events, size and site.

## ADR-0048: Store packaging, icons, a privacy policy, and a permission table that tests keep honest

**Decision.** `npm run package` builds both targets and writes the upload files (`dist/store/mohona-<target>-<version>.zip`, fixed time stamps, readable unminified code so reviewers can follow it). The Chromium upload has no pinned `key` (stores sign with their own); local builds keep it so the host id stays fixed. Firefox gets `data_collection_permissions: none`. Icons (16–128 px) are in the manifest. `docs/privacy.md` says what is handled and that nothing leaves the computer; `docs/store.md` has the listing text, a justification for every permission, reviewer notes and the submission checklist. A test fails if the manifest asks for a permission the table does not explain.

**Known consequence.** A store gives its own extension id, so the host must also be registered for that id (documented in `docs/store.md`); the store ids are not known until the first upload, so this step stays open until then.

## ADR-0049: The app is a client of the engine; the window code is a thin shell over a tested crate

**Decision.** The desktop app (`app/`) is Tauri 2 with a React, TypeScript and Tailwind page. It does not contain an engine: it talks to the same daemon as the command line and the browser extension (found through the info file, started with `mohona daemon` when not running), so downloads keep going when the window is closed and the extension and the window always show the same list. Everything the window does lives in `crates/appcore` (status, start, list with measured speed and time left, add with checks, pause, resume, cancel, remove), tested against a real in-process daemon; `app/src-tauri` only forwards eight commands. It is kept out of the engine's workspace (`exclude`), so `cargo test --workspace` does not need the system web view libraries.

**List refresh.** The page asks for the list once a second; the speed is measured by the Rust side from how much each download grew between two lists and smoothed, because the engine's API has no progress events yet. A push stream is a later improvement (it would also let the extension show live progress).

**The page can run without Tauri.** In a plain browser the page uses a pretend engine with sample downloads, which is what the page tests and design checks use. Errors from the engine reach the page as plain text and are shown as they are.

**Not done yet (next Phase 6 steps).** Queues and scheduler, speed limits and settings pages, categories and the destination folder, tray icon and notifications, clipboard watching, file open/show-in-folder buttons, language switching.

## ADR-0050: The engine's settings are typed calls, kept in the engine; the browser settings are edited in the app through the same shared text

**Decision.** The daemon has `get_config` and `set_config` (download folder, a global speed limit, and how many downloads run at once). They are separate from the free-form `extension.*` settings that clients may keep, so a client can never write the engine's own keys. `set_config` checks every value before it changes anything (the folder must be a full path and must be creatable; the number of simultaneous downloads must be 1 to 32), so a refused change is never half applied. The folder the person chose is used for every download that does not name its own folder, including those from the browser. A speed limit of 0 and an empty folder mean "remove the limit" and "back to the default". The count of simultaneous downloads is the first queue's limit (the default queue); more queues and schedules get their own page later.

**Browser settings in the app.** The settings page edits the same `extension.settings` text the browsers sync (ADR-0047): the app keeps fields it does not know, writes the ones it does, cleans site names the way the extension does, and gives the text an `updated` time strictly newer than what was there, so every browser takes the change over at its next check.

**Not included.** A folder picker (needs a dialog plugin; the folder is typed for now), proxy and connection settings, notifications, language.

## ADR-0051: Tray, messages and "Open" are done by the Rust part of the app, and never start programs

**Tray and closing.** The app has a tray icon (left click: show the window; menu: Open Mohona, Quit (downloads keep going), Quit and stop Mohona). Closing the window hides it to the tray unless the person switched "Keep Mohona in the tray" off; the Rust part decides this when the close request arrives, not the page, so a hung page cannot keep the app alive or stop it from closing. The engine is a separate process, so downloads continue either way; only "Quit and stop Mohona" stops it (running downloads are paused and resume next time).

**Messages.** The Rust part looks at the engine's list every two seconds (the page may be hidden) and shows a system message when a download finishes or fails. The first list after the app starts, or after the engine was away, only sets the starting point, so old downloads are never announced again. A failed download whose link expired says so in words. The switch "Show a message…" lives in the engine's settings (default on).

**Open and Show in folder.** Both take only the download's number; the path is looked up in the engine's own list, must belong to a finished download and must still exist. "Open" refuses programs, installers, scripts, shortcuts and macro documents (`exe`, `msi`, `bat`, `ps1`, `lnk`, `docm` and others; a trailing dot or space does not hide the type) with a reason, because a downloaded file should never be started by one click by mistake; "Show in folder" works for every file.

**Limits.** Messages on Windows show under the app's name only after it is installed (in development they appear under another name); no sound, no click-to-open yet.

## ADR-0052: The app sets the main queue's schedule and one end-of-downloads action; clients can never start programs

**Schedule.** The app edits the schedule of the default queue ("Main"): any number of windows (days, from, to, an optional speed limit while the window is open), windows may cross midnight, and an empty list means "always". The engine already applied schedules (queue closed outside windows, progress kept); the daemon gets `get_plan` and `set_plan`, which check every window before saving (a refused list changes nothing). Several queues, and choosing a queue per download, are later work: the engine has the pieces, the page does not.

**End action.** `set_after_action` arms `sleep`, `hibernate` or `shutdown` with a countdown of 5 to 3600 seconds (default 60) that starts when the last download finishes and can be cancelled; it applies once and only to downloads that finish after it was armed (as in the engine). The daemon follows the manager's events to tell the page the armed action, the seconds left, and how it ended (done, or why it failed). `run:<program>` and `exit` exist in the engine and the command line but are refused from the daemon's API: a program that can be reached by a local client, however good its token, must not be able to name a program to start.

**The page** shows the schedule's state ("outside the scheduled hours"), a banner with the armed action and its countdown with a Cancel button, and the last outcome.

**Not tested here:** the real sleep, hibernate and shutdown calls (the engine's tests replace the runner); see the manual tests.

## ADR-0053: Categories, "sort by type" and the folder dialog in the app

**Categories.** The app lists, adds and removes the categories that sort finished downloads into folders by file type (the engine's categories, the same as `mohona queue categories`). The daemon checks a category before saving it: a plain name of 1 to 40 characters without `\ / : * ? " < > |`; at least one file type, letters and digits only; no file type that another category already has (the message names that category, so one file type can never mean two folders); no name used twice (ignoring case); and a folder that is a full path or a name inside the download folder, never one that climbs out with `..`. Removing a category leaves the files already sorted where they are.

**Sorting is a setting.** "Sort finished downloads into folders by type" is a switch in the engine (default off). When it is on, every download that has no folder of its own is sorted, including those the browser extension sends. A download added with an explicit folder in the app's Add dialog goes to exactly that folder.

**Folders are chosen with the system's dialog.** Settings and the Add dialog have a "Browse…" button that opens the operating system's folder dialog through Tauri's dialog plugin; the typed path still works. A cancelled dialog changes nothing.

**Not included.** Editing a category (remove and add again), several queues, per-site folder rules.

## ADR-0054: Several queues in the app

**Queues.** The app lists, adds, renames, removes and sets queues through new daemon calls (`get_queues`, `add_queue`, `rename_queue`, `remove_queue`, `set_queue`). Each queue has its own number of simultaneous downloads (1 to 32), an optional speed limit shared by the queue, and its own schedule windows (the same editor as the main schedule). The first ("Main") queue cannot be removed; removing another moves its downloads to Main. A name is 1 to 40 characters and unique ignoring case. Every change is checked before it is applied, so a refused change leaves the queue as it was.

**Choosing a queue.** The Add dialog shows a "Queue" choice only when there is more than one queue; without a choice the download goes to Main, as the browser extension's downloads do. `add` in the daemon takes an optional `queue`.

**Not included.** Moving an existing download to another queue, and reordering downloads inside a queue (the command line can).

## ADR-0055: Live progress and connection details, categories in the list, colour themes, a new extension popup

**Progress.** The daemon's list used only the byte count the manager saved to the database once a second, and a segmented download could show a number lower than the one before. The daemon now follows the manager's progress events and answers the list with the newest count (never lower than the saved one). The engine also reports, twice a second, where every part of a segmented download is (`Event::Parts`: start, next byte, end). The daemon turns that into per-connection received bytes, speed (smoothed over a few samples) and a state (`active`, `waiting` after 3 s without bytes, `done`). A new call `get_details` returns it; a download on one connection, or one that is not running, has an empty list.

**Details window.** A "Details" button (and a double click) on running downloads opens a window that refreshes every second: totals, speed, time left, a bar of the whole file that shows which parts are received, and a table of the connections.

**Categories and sorting.** The list shows each file's category (by its extension, from the engine's categories; "Other" when none matches), the sidebar lists the categories with counts and filters by them (together with the status filter), and "Sort by" offers newest, name, size, category and type, and status. The sort is remembered.

**Themes.** Nine named themes (Desert, Forest, Sunny, Orange, Leaf, Earth, Sky and cloud, River, Water) beside System, Light and Dark. They are defined once in `design/themes.json`; `node design/themes.mjs` writes the style sheets and lists for the app and the extension and refuses a palette whose text, buttons or header text fall below WCAG contrast (4.5:1 for text, 3:1 for bars). The app and the extension each remember their own choice.

**Extension popup.** Redesigned: a header with the connection state, four counters (running, waiting, finished, failed), total speed, running downloads with progress, speed and time left, recently finished files, expired links, media on the page, and settings. The popup reads the list through a new `downloads` message and measures speed between two lists.

**Not included.** Clipboard watching and language switching (the remaining Phase 6 items) and moving a download to another queue.

## ADR-0056: Offer to download a link that was copied

**What.** A switch in Settings, "Offer to download a link when I copy one" (off by default), makes the app look at the clipboard when its window comes to the front and once at the start. If the whole text is one http, https or ftp link (no spaces, a host with a dot, at most 2048 characters) a bar offers "Download it" or "Ignore". A link is offered once: not again after Ignore or Download, and never when it is already in the list. Nothing is added without the click.

**Privacy.** The clipboard is read only while the switch is on, only by the app's own window, and only to test for a link; its content is never stored or sent anywhere except to the engine when the person presses "Download it". The switch is kept in the window's own storage, not in the engine.

**How.** Rust reads the text with `tauri-plugin-clipboard-manager` through one command, `read_clipboard`; the page does everything else. This part of the Rust shell was written without being compiled (see the manual tests).

**Not included.** Watching in the background while the window is hidden, and links inside longer text.

## ADR-0057: Languages are plain files, English is the key

**What.** The app window can be shown in 14 languages (English, Bengali, Hindi, Urdu, Arabic, Spanish, French, German, Portuguese, Russian, Chinese, Japanese, Indonesian, Turkish). A language is one JSON file in `app/src/locales`: the English text on the left, the translation on the right. The program lists every file in that folder, so adding a language means adding a file (see `docs/translating.md`); no code changes.

**Why English as the key.** A person who adds a language sees the real sentence, not a code such as `settings.title`, and a missing translation shows the English text instead of a blank or a code. `{name}` placeholders carry the variable parts, so the order of words can change in any language.

**First start.** The window uses the saved choice, else the browser's or system's language (a region such as `pt-BR` falls back to `pt`), else English. Arabic and Urdu set the page direction to right to left.

**Checks.** `npm run i18n:check` and the tests find every text in the code (including those in tables), and fail when a language lacks a text, keeps one that is gone, or changes a `{name}`. The translations that ship were written without a native speaker's review; the guide asks speakers to correct them.

**Not translated.** Messages written by the engine, names of the person's own categories and queues, the extension's background messages, and numbers' digits (Western digits everywhere).

**Extension popup (added later).** The popup uses the same scheme with its own files in `extension/src/locales` (the same 13 languages; texts it shares with the app are copied from the app's files so the wording is the same). Static page text carries `data-t` attributes and is translated when the page opens and when the language is changed; the popup keeps its own language choice and, the first time, follows the browser's language. A test fails when a language lacks a popup text or changes a `{name}`.

## ADR-0058: A modern button on videos and sounds

**Look.** The button on players is a round glass-like button with a download arrow that opens out to "Mohona" when the mouse is over it (it anchors by its right edge, so it grows to the left and stays on the player). Its card has a coloured header (the theme's gradient) with a close button, one row per found item (name, a FILE, HLS or DASH label, size) and an accent "Download" button that shows "Sending…", then a green "Sent ✓" or a red "Failed". It follows the theme and the language chosen in the popup; "System" follows the browser's light or dark mode.

**Sounds.** Sound players that show their own controls (`<audio controls>`, at least 160 by 28 pixels) get the button too, beside the bar rather than on its corner, where the player's own controls are. Hidden `<audio>` elements are page machinery and get none.

**How the choice reaches the page.** The popup saves its theme and language in the extension's local storage. When a page has a player, its script asks the background script once for a "look": the language, direction, the few translated words the button needs and the colours of the theme. The page script therefore does not carry the language files (they would be added to every page). A change in the popup reaches open pages at once.

**Still as before.** The page is only measured; nothing is read from it or sent anywhere until the person presses a button; clicks on the button never reach the page.
