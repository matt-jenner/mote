# Mote

This repository implements the catalog foundation and the second macOS desktop checkpoint for a cross-platform photo viewer. It indexes local folders and mounted network shares into a local SQLite catalog, normalizes useful metadata, tracks offline sources without discarding their records, schedules progressive background work, and manages local derivative-cache accounting.

Run the native app with the [macOS desktop development workflow](#run-the-macos-desktop-app), or package the same interface and API with the [hosted deployment guide](docs/deployment/hosted.md).

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
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

CI runs these checks, plus the 10,000-asset benchmark smoke test, on Ubuntu, macOS, and Windows.

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
per-user macOS paths below the `io.github.matt_jenner.mote` Tauri identifier:

- `~/Library/Application Support/io.github.matt_jenner.mote/profiles/<profile>/catalog.sqlite`
- `~/Library/Caches/io.github.matt_jenner.mote/profiles/<profile>/`

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

To create an unsigned macOS application bundle:

```bash
npm run desktop:build -- --bundles app
```

The bundle is written to `apps/desktop/src-tauri/target/release/bundle/macos/Mote.app`. Because it is unsigned, macOS may require you to approve it through the normal local-app security flow before first launch.

## Start the health service

The service requires explicit local directories and binds to loopback port 8080 by default:

```bash
PHOTO_VIEWER_DATA_DIR=/path/to/local/photo-viewer-data \
PHOTO_VIEWER_CACHE_DIR=/path/to/local/photo-viewer-cache \
cargo run --release -p photo-server
```

Set `PHOTO_VIEWER_BIND` only when a different socket is required. For example, `PHOTO_VIEWER_BIND=127.0.0.1:18080` keeps the service loopback-only on another port. `GET /healthz` reports database, cache, aggregate source, and warning health without exposing source paths or filenames.

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
