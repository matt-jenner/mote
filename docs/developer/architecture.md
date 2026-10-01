# Mote architecture

Mote keeps source access, catalogue state, generated files, and presentation
separate. Desktop and hosted builds share the React interface and Rust service
code while using different folder-selection and persistence adapters.

## Repository map

| Path | Responsibility |
| --- | --- |
| `apps/interface` | React and TypeScript gallery, viewer, Picks, and service adapters |
| `apps/desktop` | Tauri desktop shell and native commands |
| `crates` | Rust domain, catalogue, indexing, metadata, cache, and server code |
| `deploy` | Compose and reverse-proxy examples |
| `packaging` | HEIC source builds and Linux Flatpak packaging |
| `scripts` | Verified build, cleanup, and smoke-test entry points |
| `tests` | Browser, build, deployment, and packaging contracts |

## Rust crates

- `photo-domain` defines stable IDs, path keys, media classes, and folder
  policies.
- `photo-catalog` owns SQLite migrations and repositories.
- `photo-core` coordinates library lifecycle and folder policies.
- `photo-metadata` reads EXIF, XMP, geometry, colour, and provenance.
- `photo-indexer` scans sources, schedules work, and reconciles changes.
- `photo-cache` manages content-addressed derivatives, atomic writes, repair,
  and eviction.
- `photo-app-service` exposes application operations shared by adapters.
- `photo-server` serves the hosted API, web assets, health, and local state.
- `photo-codec` decodes supported image formats, including optional HEIC.
- `catalog-bench` measures large synthetic catalogue operations.

## Data boundaries

Photo sources are read-only. SQLite state belongs in the application data
directory, and generated JPEG derivatives belong in the cache directory. A
hosted browser receives root-relative identities rather than native host paths.

The desktop adapter persists saved folders and Picks in SQLite. The hosted
adapter keeps saved folders, Picks, and preferences in browser storage for each
site and browser profile.

## Background work

Indexing records photos first, then schedules wall thumbnails before larger
screen previews. The interface can display provisional rows while metadata and
derivatives settle. A failure belongs to the affected asset and must not block
the rest of the source.

Offline sources remain in the catalogue. Cached derivatives remain readable,
but Mote does not generate missing derivatives until the source returns.

## Benchmark

Run the deterministic 10,000-asset smoke profile:

```bash
cargo run -p catalog-bench --release -- \
  --assets 10000 --output target/catalog-benchmark-smoke.json
```

Use `1000000` assets for the full local profile. The report records catalogue
size and timings without enforcing hardware-specific limits.
