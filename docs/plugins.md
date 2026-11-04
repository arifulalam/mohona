# Plugins

A plugin teaches Mohona about one site: given a link of that site, it says which address to download. Typical uses: a download page whose real file link is built from a token, or a site that needs a special header.

## Where they live

`mohona plugins list` prints the plugins folder (next to the daemon's files, folder `plugins`). Each plugin is a folder in it:

```
plugins/
  my-site/
    plugin.json
    resolve.ps1      (or any program you like)
```

An example is in `examples/plugins/rewrite-demo` in the source.

## `plugin.json`

```json
{
  "name": "My site",
  "version": "1",
  "description": "What it does",
  "hosts": ["files.example.com", "*.cdn.example.com"],
  "command": ["powershell", "-NoProfile", "-File", "resolve.ps1"]
}
```

- `name`: 1 to 40 letters, digits, spaces, `_`, `-`, `.`. It is what you switch on with `mohona plugins enable "My site"`.
- `hosts`: the sites it handles (the same patterns as rules: `example.com` includes subdomains, `*.example.com` only subdomains).
- `command`: the program and its arguments, up to 8 words, **no shell**. The program is a name on the PATH (`python`, `powershell`, `node`) or a path inside the plugin's folder. Absolute paths, drive letters and `..` are refused.

## What a plugin does

Mohona runs the command in the plugin's folder with the link added as the **last argument**, waits at most 30 seconds, and reads one JSON object from standard output (at most 64 KiB):

```json
{ "url": "https://cdn.example.com/real/file.zip",
  "referrer": "https://example.com/page",
  "user_agent": "optional",
  "headers": { "X-Token": "abc" } }
```

Only `url` is required, and it must be an http or https address. Headers may not be cookies, logins or anything that breaks a request (the same list as site profiles), at most 10. A plugin that fails, prints something else, or takes too long stops the add with a message that names it; the link is not downloaded as a web page by mistake.

## Safety

- A plugin is a program that runs with your rights. Install only plugins you trust, and read what they do.
- Plugins are **off** until you switch them on: `mohona plugins enable NAME`. Nothing runs for a site you did not switch on.
- The program starts with almost no environment (PATH, temp folders, home, language) so it does not see Mohona's settings; it gets no input but the link, and no console window opens on Windows.
- The page's login cookies are not passed to the plugin. If the real address is on another site, they are not sent there either.
- Rules and site profiles are matched against the link you pasted; the download uses the plugin's address.
- Plugins are not synced between computers (`mohona sync` never carries programs).

## Not included

Plugins that add buttons or windows to the app, plugins that download by themselves (they only name the address), a catalogue or automatic updates, and the YouTube/yt-dlp handling, which has its own support (`mohona stream`).
