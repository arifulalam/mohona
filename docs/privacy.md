# Mohona privacy policy

*Applies to the Mohona browser extension and the Mohona download manager. Last changed: 2026-10-26. Project contact: see the repository's issue tracker.*

**Short version: Mohona has no servers, no accounts, no ads and no analytics. Nothing about you or your browsing is sent to the Mohona project or to anyone else.**

## What the extension handles, and where it goes

| What | Why | Where it goes |
| --- | --- | --- |
| The address of a download, the page it came from, your browser's user agent, and the cookies for the download's site | So the download manager can fetch a file that needs your login, exactly like your browser would | Only to the Mohona program on **your own computer** (through the browser's native messaging, and then a local connection on 127.0.0.1). The cookies are used for that one download and are not copied anywhere else. |
| The addresses of video and audio files and streams a page loads | To list them in the extension's popup and on the video button so you can pick one | Kept in the browser's temporary session storage for that tab; erased when the tab or browser closes. Sent to Mohona on your computer only when you choose to download one. |
| Files held by the page (`blob:`, `data:`) that you save from the right-click menu | To save them | Only to Mohona on your computer. |
| Your extension settings (take over downloads, minimum size, excluded sites, video button, hidden sites) | To work the way you chose | Your browser's sync storage (if you are signed in to the browser, the browser vendor may sync it, as for every extension) and a copy in the Mohona program on your computer. |

The extension never contacts a server of its own. The Mohona program downloads the files **you** ask for from the sites they are on, and nothing else. Mohona does not read, store or send what you type in pages, your history, your passwords or the contents of pages. The video button reads only the size and position of video players and their address, and does nothing until you click it.

Downloads in private (incognito) windows are never taken over.

## What is stored

On your computer, Mohona keeps its list of downloads (address, name, status, dates) in a database next to the program, and the files you download where you told it. You can delete them at any time, and uninstalling removes the program. The extension keeps its settings in the browser; removing the extension removes them.

## Sharing

None. There is no third party, no telemetry, no crash reports and no selling or sharing of data.

## Children

Mohona does not collect data from anyone, including children.

## Changes

Changes to this policy are published with the source code (`docs/privacy.md`), with the date above.
