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
