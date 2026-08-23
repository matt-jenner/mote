# Photo Viewer

This repository currently implements the headless catalog foundation for a cross-platform photo viewer. It indexes local folders and mounted network shares into a local SQLite catalog, normalizes useful metadata, tracks offline sources without discarding their records, schedules progressive background work, and manages local derivative-cache accounting.

Source media is read-only. Production code never writes, renames, or deletes files under a configured photo root. SQLite state and generated derivatives stay in explicit local data and cache directories; do not place either directory inside a photo source.

The desktop and web interfaces are deliberately outside this foundation slice.

## Requirements

- Rust 1.97.1 installed through [rustup](https://rustup.rs/)
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
