# Publishing Mohona

Where things go: the **source** is on GitHub (`arifulalam/mohona`), the **Windows installer** is a GitHub Release, and the **Rust crates** (so that `cargo install mohona` works) are on crates.io. The extension store pages are separate (see `extension/README.md`).

## crates.io

What is published: `mohona` (the command-line program), `mohona-host` (the browser extension's native host) and the libraries they stand on: `mohona-media`, `mohona-net`, `mohona-store`, `mohona-core`, `mohona-grabber`, `mohona-torrent`, `mohona-manager`, `mohona-daemon`. Not published: `mohona-appcore` (only the desktop app uses it) and the test server.

The crate names `mohona` and the `mohona-*` ones must be yours on crates.io; `mohona` is already reserved for you. A published version can never be changed or removed (only "yanked"), so publish only a version you have tested.

1. Pick the version. All crates share one number: `version` in the root `Cargo.toml` (`[workspace.package]`), and each `version = "…"` next to an internal `path = …` dependency must say the same (search for `mohona-` in `crates/*/Cargo.toml`). Move to `0.1.0` once the Windows tests pass.
2. Log in once: make a token at https://crates.io/settings/tokens (scope: publish-new and publish-update) and run `cargo login`.
3. Check everything (nothing is uploaded):
   ```
   cargo publish --workspace --exclude mohona-appcore --dry-run
   ```
   It packs every crate in the right order and builds each from its packed copy; every crate must say "aborting upload due to dry run". Commit or stash first so that the packed copy is what you have in git (add `--allow-dirty` only for a trial).
4. Publish for real, all in the right order, in one command:
   ```
   cargo publish --workspace --exclude mohona-appcore
   ```
   If it stops half way (network), run it again: the crates already published are skipped.
5. Try it: `cargo install mohona` in a clean place, then `mohona --version`.
6. Tag the same version on GitHub (`git tag v0.1.0 && git push --tags`) and create the release there with the installer (`app\src-tauri\target\release\bundle\nsis\*.exe`) attached. The app's "Check for updates" (Settings → General) reads this release, so the tag must be the version number (with or without `v`).

## After a mistake

`cargo yank --version 0.1.0 mohona` stops new projects from picking that version; fix, raise the version and publish again.

## Crediting Claude

GitHub lists a co-author when a commit message ends with a blank line and `Co-authored-by: Claude <noreply@anthropic.com>`. Claude cannot be added as a repository collaborator, and crates.io owners must be real accounts, so on crates.io credit Claude in the README only; keep `authors` as you.
