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
| `--ca-file FILE` | Also trust the certificate authority in this PEM file (a server with its own certificate). The full path is saved with the download |
| `--insecure` | Accept any certificate, even wrong or expired. Unsafe: only for a server you know |

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

### Errors you may see

| Message | What it means / what to do |
|---|---|
| `the server answered 401 ...: it asks for a login (Basic)` | Add `-u user:password` |
| `the server answered 401 ...: it refused the user name or password` | Wrong login |
| `the server answered 401` with a note that it sent no login method | The site uses a web login page, not HTTP login. Log in in a browser and pass its cookie with `--cookie` |
| `the server answered 404 ...` | The address is wrong or the file is gone |
| `the server answered 500 ...` | The server itself failed; try later |
| `unsupported URL scheme "ftp" (only http and https work so far)` | FTP/SFTP come in a later phase |
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
