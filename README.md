# Mote

This repository implements a cross-platform photo viewer for local folders and mounted network shares. It indexes photos into a local SQLite catalog, normalizes useful metadata, tracks offline sources without discarding their records, schedules progressive background work, and manages local derivative-cache accounting.

Build the native app with the [Windows, macOS, and Linux build guide](docs/deployment/building.md), or package the same interface and API with the [hosted deployment guide](docs/deployment/hosted.md).

Source media is read-only. Production code never writes, renames, or deletes files under a configured photo root. SQLite state and generated derivatives stay in explicit local data and cache directories; do not place either directory inside a photo source.

## Mote brand assets

The approved Mote identity, fonts, logos, application icons, PWA assets, and print files live in [`docs/brand/`](docs/brand/). Start with the [brand asset and usage guide](docs/brand/README.md).

Common downloads:

- [Horizontal logo for light backgrounds](docs/brand/svg/mote-lockup-horizontal-light.svg)
- [Horizontal logo for dark backgrounds](docs/brand/svg/mote-lockup-horizontal-dark.svg)
- [macOS application icon](docs/brand/icons/macos/Mote.icns)
- [Windows application icon](docs/brand/icons/windows/Mote.ico)
- [Linux application icons](docs/brand/icons/linux/hicolor/)
- [Hosted web and PWA icons](docs/brand/icons/web/)
- [Printable brand sheet](docs/brand/print/mote-brand-sheet-a4.pdf)

## Requirements

- Rust 1.97.1 installed through [rustup](https://rustup.rs/)
- Node.js 24.18.0 and npm 11.16.0 or newer
- A local filesystem location for SQLite state
- A separate local filesystem location for derivative cache data

The checked-in `rust-toolchain.toml` selects Rust 1.97.1 with Rustfmt and Clippy.

## Verify the workspace

Run the same checks used by CI:

```bash
npm run rust:verify
```

CI runs these checks, plus the 10,000-asset benchmark smoke test, on Ubuntu, macOS, and Windows.
The managed local command shares one temporary Cargo target across the checks
and removes it afterward, including after a failed or interrupted run. Direct
Cargo commands can leave large `target/` trees; `npm run clean:build` removes
those known repository leftovers and `build/heic-native`, including its
download cache and partial staging trees. The `clean:build-assets` alias does
the same job. Neither command touches source media, retained packages, the rest
of `dist/`, or application data.

## Choose the HEIC build mode

Normal builds include HEIC and HEIF decoding. Pass one `--no-heic` immediately
after the Mote build command to omit only HEIC support while retaining every
other default feature:

```bash
npm run desktop:build
npm run desktop:build -- --no-heic
npm run desktop:build:windows
npm run desktop:build:windows -- --no-heic
npm run flatpak -- package
npm run flatpak -- package --no-heic
scripts/hosted-smoke.sh
scripts/hosted-smoke.sh --no-heic
```

The build helpers consume this flag before passing any remaining arguments to
Tauri, Flatpak Builder, or the selected container engine.

Enabled release packages include the libheif/libde265 notices, full LGPL text,
verified source pins, and shared-library replacement instructions. The
repository copies are [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and the
[HEIC native decoder source and replacement guide](packaging/heic/README.md).
Mote's own licence remains separate and unchanged.

## Install and verify the interface

Install the locked Node dependencies and Playwright's browser for the current
platform. Browser tests use WebKit on macOS and Chromium elsewhere:

```bash
npm ci
if [[ "$(uname -s)" == "Darwin" ]]; then
  npm exec playwright install webkit
else
  npm exec playwright install chromium
fi
```

On Arch Linux, the browser tests use the system Chromium at
`/usr/bin/chromium` when it is installed. Set `MOTE_CHROMIUM_PATH` to use a
different system Chromium executable.

Run the interface checks:

```bash
npm run check
npm run typecheck
npm test
npm run test:browser
npm run --workspace @photo-viewer/interface build
```

## Keep and review Picks

Use a photo's pick control to keep it in Picks. Picks collect photos across
saved folders in the order you add them. Review picks opens that sequence;
changing folders or removing a saved folder shortcut does not remove its picks.
Unavailable photos stay listed with a warning, and cached previews remain
viewable.

Desktop Picks live in the local catalogue and survive app restarts within the
same profile. Hosted Picks live in this browser's local storage, separately for
each server root. Tabs on the same site update when another tab changes Picks.
Clearing site data removes the hosted list; a different browser has its own
list. If storage is unavailable, Picks remain usable in the current tab and a
warning explains that they cannot be saved.

Remove a row to drop one pick. When no copy attempt is active, **Clear picks**
empties the list immediately and offers **Undo** for five seconds. Undo restores the cleared sequence ahead of
photos added since Clear, without duplicates. Closing Picks, reviewing photos,
copying, and downloading do not clear the list.

On desktop, **Copy originals…** opens the destination picker before preparing
the batch. Dismissing the picker is silent. Each original is written to a private
temporary file in the destination, then published under its final name only
after all bytes are complete. Existing files are never overwritten; name
collisions receive a numbered suffix. Mote remembers the
destination for the next picker after a successful copy. **Show folder** opens
the completed copy's destination. Partial failures remain visible on their
rows; Retry opens the picker again and retries only failed originals.

Copy progress appears only in the Picks drawer. **Cancel** replaces Clear while
copying, and **Cancelling...** remains disabled until cleanup finishes. Cancel
keeps completed files and picks, removes the current temporary file, restores
the drawer's previous copy state, and shows only a fading **Copy cancelled**
toast lasting at most one second. Closing the drawer leaves the copy running.
If the destination disappears, the batch stops with **The destination folder
no longer exists.** Mote does not remove completed copies. Adding or removing a
pick does not change a batch already running. Source photos remain untouched.
After a completed attempt, Clear also removes copy messages and row errors.

Hosted sites offer a **Download original** link on each available row only when
the administrator enables `PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS=true` and
restarts the server or container. The default is off. Each link downloads that
one original; opening Picks and reviewing previews never downloads originals.

To verify the hosted Picks workflow locally, install Playwright Chromium, build
the web app, then run:

```bash
npm run web:build
npm run test:hosted -- --grep Picks
```

The tests start two temporary loopback servers with disposable source copies,
separate catalogues and caches, and downloads off/on. They require Cargo and
built web assets. The broader container lifecycle smoke test is documented in
the [hosted deployment guide](docs/deployment/hosted.md).

## Run the macOS desktop app

For normal desktop development, use the default profile:

```bash
npm run desktop:dev
```

The default profile keeps its SQLite catalog and generated cache under the operating system's standard application data and cache directories. For a clean, isolated run, name a profile:

```bash
PHOTO_VIEWER_PROFILE=clean-demo npm run desktop:dev
```

A named profile isolates the local SQLite catalog and generated derivatives in
per-user macOS paths below the `io.github.matt-jenner.mote` Tauri identifier:

- `~/Library/Application Support/io.github.matt-jenner.mote/profiles/<profile>/catalog.sqlite`
- `~/Library/Caches/io.github.matt-jenner.mote/profiles/<profile>/`

For the progressive wall demonstration, use a fresh profile and choose the
checked-in `apps/interface/public/demo-photos` folder in the native picker:

```bash
PHOTO_VIEWER_PROFILE=wall-demo npm run desktop:dev
```

Use another profile name, such as `wall-demo-2`, for another clean run without
reusing the first catalog or derivative cache. No command in this workflow
copies, renames, deletes, or otherwise modifies photos in the selected source
folder.

The automated source-safety check opens that demo folder read-only and keeps its
catalogue and derivative cache in a temporary directory outside the source
tree. It verifies the source bytes and modification times before and after
scan, cache repair, derivative phases, sorting, viewer-equivalent requests,
restart, and offline cached reads.

The catalogue retains indexed videos, but photo surfaces hide them without a
placeholder or hidden count until cross-platform video playback ships. A wall
tile becomes openable only after its current thumbnail has painted. Background
work runs in order: wall thumbnails first, then screen previews. A corrupt
photo records its failure and does not block healthy photos.

The gallery initially includes photos from the selected folder and every
subfolder. Use **Include subfolders** in the wall toolbar to switch to photos
stored directly in the selected folder; the choice persists across restarts.
Changing the scope reprojects the local catalogue and cached derivatives, so it
does not rescan the source or wait for a mounted network share.

When a wall tile opens the immersive viewer, use Left/Right (or the Previous
and Next buttons) to move through the loaded order. Select a filmstrip item for
a direct jump, or swipe horizontally on a fit-to-window photo. Photo
information opens the read-only metadata drawer, and Back to photos or Escape
unwinds the viewer state in order: it closes Info, resets zoom to Fit, then
returns to the wall and restores the selected tile. Ready cached derivatives
remain usable if the selected source becomes unavailable. The viewer stays
dark when the system appearance is light. In the viewer, use
Plus/Minus or `+`/`-` to zoom around the viewport centre, `0` or Fit to return
to the fitted image, and drag a zoomed photo to pan. Double-click or double-tap
toggles between Fit and the derivative's native 100 percent limit; trackpad
pinch zooms around its midpoint. A contextual navigator appears for desktop
pointer input while zoomed, and the filmstrip, keyboard, and navigation buttons
remain available for photo changes.

Zoom and pan are cache-only. The viewer never reads or enlarges the original
source file, and it caps zoom at the best decoded derivative's native pixel
density. If the source becomes unavailable, a cached wall thumbnail or screen
preview can still be viewed and zoomed; an uncached derivative has no in-viewer
recovery path until desktop locate/reconnect exists.

An unavailable catalog item with no cached derivative does not open in this
viewer: `PhotoService` has no locate-folder capability yet. Desktop
locate/reconnect is a named follow-up, and the hosted web interface must not
offer local folder selection. Until that capability exists, such a desktop
item has no in-viewer recovery path.

To create an ad-hoc-signed macOS application bundle:

```bash
npm run desktop:build
```

The bundle is written to `dist/macos/Mote.app`. A successful build replaces the
previous repository copy only after the new bundle has been validated. All
Cargo intermediates are then removed. The ad-hoc signature protects the bundle
from accidental changes, but it does not identify an Apple Developer account and
the app is not notarized. macOS may therefore require approval before first launch.

To build one `.app` containing both Apple Silicon and Intel executables, run
these commands on a Mac with the Xcode Command Line Tools installed:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm ci
npm run desktop:build:universal
```

The universal bundle also replaces `dist/macos/Mote.app` only after validation.
The Intel executable supports the architecture used by 2019 MacBook Pro models.
The bundle's deployment target is macOS 11.0, but the interface uses modern
WebKit features and has not been verified on older macOS/Safari versions. Use
an up-to-date macOS version supported by the destination Mac. Verify the
bundle's architectures with:

```bash
lipo -archs dist/macos/Mote.app/Contents/MacOS/photo-viewer-desktop
```

The output must include both `x86_64` and `arm64`. To create the same kind of disk
image used for releases:

```bash
version="$(node -p 'require("./apps/desktop/src-tauri/tauri.conf.json").version')"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
ditto dist/macos/Mote.app "$stage/Mote.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname Mote -srcfolder "$stage" -format UDZO \
  "dist/macos/Mote-${version}-macOS.dmg"
hdiutil verify "dist/macos/Mote-${version}-macOS.dmg"
rm -rf "$stage"
trap - EXIT
```

Open the DMG and drag `Mote.app` into Applications. All distributed macOS builds
are intentionally ad-hoc signed rather than Developer ID signed or notarized, so
Gatekeeper's first-launch warning is expected. See the
[macOS installation guide](docs/deployment/macos.md) for the safe approval steps.

The separate [macOS workflow](.github/workflows/build-macos.yml) runs only when a
GitHub Release is published or when it is deliberately dispatched by hand. One
job builds the HEIC-enabled universal app, verifies both architectures and its
ad-hoc signature, mounts and checks the DMG, then attaches the DMG to the release.
A manual check retains its DMG as an Actions artifact for one day. Pushes and pull
requests do not consume macOS Actions minutes, and local builds use none.

## Build the Windows installer

Windows x64 builds use the pinned MSVC HEIC decoder path and produce an NSIS
installer at `dist/windows/Mote-<version>-windows-x64-setup.exe`:

```powershell
npm ci
npm run desktop:build:windows
```

Run this from an x64 MSVC developer shell with PowerShell 7, CMake, Rust 1.97.1,
Node.js 24.18.0, and npm 11.16.0 available. The builder verifies the executable
and decoder DLLs, removes Cargo and interface intermediates on success or
failure, and replaces the prior installer only after validation. The native
decoder cache remains bounded to the pinned archives and accepted prefix so a
later build can reuse it.

Windows release installers are unsigned. Microsoft Defender SmartScreen may
therefore require **More info > Run anyway** on first installation. Only approve
an installer downloaded from this project's official GitHub Release or built
from a reviewed checkout. See the [complete platform build guide](docs/deployment/building.md)
for prerequisites, cleanup, release behavior, and the VM acceptance checklist.

The [Windows release workflow](.github/workflows/build-windows.yml) runs only for
a published GitHub Release or a deliberate manual dispatch. It creates one
HEIC-enabled x64 installer. Release runs attach it to the release; manual checks
retain it as an Actions artifact for one day. Pushes and pull requests do not
consume Windows Actions minutes.

The permanent identifier `io.github.matt-jenner.mote` changes the macOS data
and cache directories from those used by older `app.photoviewer.desktop`
builds. Existing catalogs are not migrated automatically, so an upgrade from
the old identifier initially opens a fresh catalog. The old data is retained.

## Build and run the Linux Flatpak

Mote's distribution-independent Linux package is built from source with
`flatpak-builder` and uses portal-only folder access. Follow the
[Flatpak build and installation guide](packaging/flatpak/README.md) to create a
local `.flatpak` bundle, install it on this workstation, or copy it to Fedora.

## Start the health service

The service requires explicit local directories and binds to loopback port 8080 by default:

```bash
PHOTO_VIEWER_DATA_DIR=/path/to/local/photo-viewer-data \
PHOTO_VIEWER_CACHE_DIR=/path/to/local/photo-viewer-cache \
cargo run --release -p photo-server
```

Set `PHOTO_VIEWER_BIND` only when a different socket is required. For example, `PHOTO_VIEWER_BIND=127.0.0.1:18080` keeps the service loopback-only on another port. `GET /healthz` reports database, cache, aggregate source, and warning health without exposing source paths or filenames.

## Hosted and Docker configuration

| Variable | Default | Accepted values | Effect |
| --- | --- | --- | --- |
| `PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS` | off | empty, `0`, `false`, `1`, or `true`, with ASCII case ignored for `true` and `false` | When on, the hosted bootstrap advertises original-download support. Restart the server or container after changing it. |

The setting is disabled unless explicitly enabled. Any other non-empty value stops startup with a configuration error.

PowerShell uses the same variables:

```powershell
$env:PHOTO_VIEWER_DATA_DIR = "C:\PhotoViewer\Data"
$env:PHOTO_VIEWER_CACHE_DIR = "C:\PhotoViewer\Cache"
cargo run --release -p photo-server
```

Use the release profile when viewing real collections. Image decoding, resizing,
and JPEG encoding are intentionally CPU-heavy and are substantially slower in an
unoptimised development build. The container image already builds and runs this
optimised release binary.

## Run the catalog benchmark

The benchmark deterministically generates catalog rows in 500-asset
transactions. The million-asset profile contains 90 percent stills and 10
percent videos, all shaped in one folder group. Videos stay indexed but are
filtered from photo pages. The run marks the generated root offline while
retaining every asset, then measures insertion, the first and second 100-photo
wall pages, a 250-ID coordinator page, a current terminal-failure lookup,
unavailable-asset counting, and cache-group eviction planning. It records
timings without enforcing hardware-dependent limits.

Run the million-asset profile in release mode:

```bash
cargo run -p catalog-bench --release -- --assets 1000000 --output target/catalog-benchmark.json
```

For a quick local smoke run:

```bash
cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-smoke.json
```

The JSON report includes `assets`, `sqlite_version`, `database_bytes`,
`insert_ms`, `first_page_ms`, `first_page_rows`, `second_page_ms`,
`second_page_rows`, `coordinator_page_ms`, `coordinator_page_rows`,
`terminal_lookup_ms`, `unavailable_count_ms`, and `eviction_plan_ms`.

## Foundation crates

- `photo-domain`: stable IDs, path keys, media classification, and folder-policy types
- `photo-catalog`: SQLite migrations and catalog repositories
- `photo-core`: library lifecycle and per-library folder policies
- `photo-metadata`: EXIF, XMP, shape, colour, and provenance readers
- `photo-indexer`: progressive scanning, scheduling, reconciliation, and watcher hints
- `photo-cache`: content-addressed keys, atomic cache writes, repair, and group eviction
- `photo-server`: loopback health service and local-state configuration
- `catalog-bench`: deterministic large-catalog measurement tool
