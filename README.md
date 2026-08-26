# Photo Viewer

This repository implements the catalog foundation and the second macOS desktop checkpoint for a cross-platform photo viewer. It indexes local folders and mounted network shares into a local SQLite catalog, normalizes useful metadata, tracks offline sources without discarding their records, schedules progressive background work, and manages local derivative-cache accounting.

Source media is read-only. Production code never writes, renames, or deletes files under a configured photo root. SQLite state and generated derivatives stay in explicit local data and cache directories; do not place either directory inside a photo source.

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

Install the locked Node dependencies and Playwright's WebKit browser:

```bash
npm ci
npm exec playwright install webkit
```

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
per-user macOS paths below the `app.photoviewer.desktop` Tauri identifier:

- `~/Library/Application Support/app.photoviewer.desktop/profiles/<profile>/catalog.sqlite`
- `~/Library/Caches/app.photoviewer.desktop/profiles/<profile>/`

For the progressive wall demonstration, use a fresh profile and choose the
checked-in `apps/interface/public/demo-photos` folder in the native picker:

```bash
PHOTO_VIEWER_PROFILE=wall-demo npm run desktop:dev
```

Use another profile name, such as `wall-demo-2`, for another clean run without
reusing the first catalog or derivative cache. No command in this workflow
copies, renames, deletes, or otherwise modifies photos in the selected source
folder.

When a wall tile opens the immersive viewer, use Left/Right (or the Previous
and Next buttons) to move through the loaded order. Select a filmstrip item for
a direct jump, or swipe horizontally on a fit-to-window photo. Photo
information opens the read-only metadata drawer, and Back to photos or Escape
returns to the wall and restores the selected tile. Ready cached derivatives
remain usable if the selected source becomes unavailable. Zoom and pan are not
part of this slice.

An unavailable catalog item with no cached derivative does not open in this
viewer: `PhotoService` has no locate-folder capability yet. Desktop
locate/reconnect is a named follow-up, and the hosted web interface must not
offer local folder selection. Until that capability exists, such a desktop
item has no in-viewer recovery path.

To create an unsigned macOS application bundle:

```bash
npm run desktop:build -- --bundles app
```

The bundle is written to `apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`. Because it is unsigned, macOS may require you to approve it through the normal local-app security flow before first launch.

## Start the health service

The service requires explicit local directories and binds to loopback port 8080 by default:

```bash
PHOTO_VIEWER_DATA_DIR=/path/to/local/photo-viewer-data \
PHOTO_VIEWER_CACHE_DIR=/path/to/local/photo-viewer-cache \
cargo run -p photo-server
```

Set `PHOTO_VIEWER_BIND` only when a different socket is required. For example, `PHOTO_VIEWER_BIND=127.0.0.1:18080` keeps the service loopback-only on another port. `GET /healthz` reports database, cache, aggregate source, and warning health without exposing source paths or filenames.

PowerShell uses the same variables:

```powershell
$env:PHOTO_VIEWER_DATA_DIR = "C:\PhotoViewer\Data"
$env:PHOTO_VIEWER_CACHE_DIR = "C:\PhotoViewer\Cache"
cargo run -p photo-server
```

## Run the catalog benchmark

The benchmark deterministically generates catalog rows in 500-asset transactions, marks the generated root offline while retaining every asset, then measures insertion, the first natural-path page, unavailable-asset counting, and cache-group eviction planning. It records timings without enforcing hardware-dependent limits.

Run the million-asset profile in release mode:

```bash
cargo run -p catalog-bench --release -- --assets 1000000 --output target/catalog-benchmark.json
```

For a quick local smoke run:

```bash
cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-smoke.json
```

The JSON report includes `assets`, `sqlite_version`, `database_bytes`, `insert_ms`, `first_page_ms`, `first_page_rows`, `unavailable_count_ms`, and `eviction_plan_ms`.

## Foundation crates

- `photo-domain`: stable IDs, path keys, media classification, and folder-policy types
- `photo-catalog`: SQLite migrations and catalog repositories
- `photo-core`: library lifecycle and per-library folder policies
- `photo-metadata`: EXIF, XMP, shape, colour, and provenance readers
- `photo-indexer`: progressive scanning, scheduling, reconciliation, and watcher hints
- `photo-cache`: content-addressed keys, atomic cache writes, repair, and group eviction
- `photo-server`: loopback health service and local-state configuration
- `catalog-bench`: deterministic large-catalog measurement tool
