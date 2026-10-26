# Publishing the extension to the stores

Build the upload files: `cd extension && npm install && npm run package`. This writes `extension/dist/store/mohona-chromium-<version>.zip` (Chrome Web Store and Edge Add-ons) and `mohona-firefox-<version>.zip` (Firefox Add-ons). The Chromium file has no `key` (the store gives the extension its own id); see "The extension id" below.

## The extension id (read before the first upload)

The host only accepts the extension with the id it was registered for (`CHROMIUM_ID` in `crates/host/src/register.rs`, `identity.json`). A store assigns **its own** id to the listing, and it will differ from the test id used while developing. After the first upload:

1. Take the id the store shows (Chrome Web Store: the item's address; Edge: its product id).
2. Add it to the Chromium ids that `mohona-host install` registers (the list in `register.rs`, then rebuild the host). Edge and Chrome each get their own id.
3. Alternatively keep the pinned test id: Chrome Web Store accepts an upload whose manifest keeps the `key` only for the first upload of an item created with that key; do this only if the store's own instructions at the time say so.

Firefox keeps its fixed id `mohona@mohona.local` (`identity.json`); change it to an address you own before the first upload (it cannot be changed later), then rebuild.

## Store listing text

**Name:** Mohona. **Short description (up to 132 characters):** Send downloads and videos from your browser to the Mohona download manager: fast, resumable, free.

**Long description:**

> Mohona is a free, open-source download manager for your computer. This extension connects your browser to it.
>
> • Takes over browser downloads (and gives them back to the browser whenever Mohona is not running).
> • "Download with Mohona" in the right-click menu for links, images, video and audio.
> • Finds videos and audio on a page, including HLS and DASH streams, and shows a small download button on video players (it can be hidden per site).
> • Saves files that only the browser holds (blob: and data: addresses).
> • Renews expired download links from the page they came from.
> • Private windows are never touched.
>
> Needs the free Mohona program on your computer (install it, then run `mohona-host install`). No account, no ads, no tracking: see the privacy policy.

Category: Productivity (Chrome), Tools (Firefox). Single purpose (Chrome Web Store field): *Send downloads and media from the browser to the Mohona download manager on the user's computer.*

## Why each permission

| Permission | Justification to paste into the store form | Where it is used |
| --- | --- | --- |
| `nativeMessaging` | Talks to the Mohona program installed on the user's computer. This is the extension's only channel; it has no server. | `src/native.ts` |
| `downloads` | Sees downloads the browser starts, so Mohona can take them over, and cancels the browser's copy once Mohona has it. | `src/intercept.ts` |
| `cookies` | Reads the cookies of a download's site so Mohona can fetch files that need the user's login. Sent only to Mohona on the user's computer. | `src/background.ts` |
| `storage` | Keeps the extension's settings and the list of media seen in a tab (session only). | `src/settings.ts`, `src/background.ts` |
| `contextMenus` | The "Download with Mohona" right-click entry. | `src/menu.ts` |
| `webRequest` (read only, no blocking) | Looks at response headers (type and size) to list video and audio files and streams a page loads. It never blocks or changes a request. | `src/sniffer.ts` |
| `scripting` | Reads the addresses on a page (and the content of a `blob:` the user chose to save) when the user asks for it, and to find a fresh link for an expired download. | `src/local.ts`, `src/refresh.ts` |
| `alarms` | A once-a-minute check that renews expired links and syncs settings, while Mohona is running. | `src/background.ts` |
| Host access `<all_urls>` / content script on `http(s)` pages | Downloads and videos can be on any site, and the video button must appear on any page with a video. The script reads only the size, position and address of video players, and sends nothing until the user clicks. | `src/content.ts` |

Do **not** add permissions without adding a row here: `tests/store.test.ts` fails when the manifest asks for something this table does not mention.

## Notes for reviewers (paste into "notes to reviewer" / "test instructions")

> The extension needs the Mohona desktop program (https://github.com/…/mohona, MIT or Apache-2.0) for most functions; without it the popup says "Mohona is not running" and the browser downloads normally. To test: install Mohona, run `mohona-host install`, `mohona daemon`, then use the popup or right-click a link. Source is unminified (`npm run build`); the store zip is exactly `npm run package`'s output. No remote code, no analytics, no network requests from the extension itself.

Replace the repository address once it is public.

## Checklist before submitting

- [ ] Repository is public, with the privacy policy at a stable address (`docs/privacy.md` on the main branch) — the stores ask for its URL.
- [ ] Screenshots: 1280×800 (Chrome, Edge) of the popup, the video button panel and a download in Mohona; Firefox takes the same.
- [ ] Promotional tile 440×280 (Chrome, optional), icon 128×128 (already in the zip).
- [ ] Chrome Web Store: developer account (one-time fee), fill "privacy practices" from the table above, data use: *no user data collected* is **not** accurate for cookies and web history handled locally; declare "Web history" and "Website content" as handled and answer that nothing is sold, shared or used for anything but the single purpose (they stay on the user's computer).
- [ ] Edge Add-ons: free developer account; same zip and text.
- [ ] Firefox Add-ons (AMO): upload the Firefox zip and the source (the repository, or `git archive`) with build steps (`npm install && npm run package`, Node 22). Listed or self-distributed.
- [ ] The extension id of each store is registered in the host (see above), and the installer of the app (Phase 8) calls `mohona-host install`.
- [ ] Version in `manifest.base.json` raised for every upload (stores refuse the same version twice).
