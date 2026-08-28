# Task 7 report: coordinated photo-loading verification

Date: 2026-08-28
Branch: `codex/viewer-zoom-pan`

## Scope

Task 7 adds automated benchmark coverage and records the verification evidence
for the coordinator, photo-only projections, viewer, and desktop bundle. No
desktop development process was started. No merge, push, worktree deletion, or
native user-acceptance action was performed.

## RED/GREEN evidence

The benchmark smoke test was changed first. The first run failed because the
new report fields did not exist:

```text
cargo test -p catalog-bench --test benchmark_smoke
error[E0609]: no field `second_page_ms` on type `BenchmarkReport`
error[E0609]: no field `second_page_rows` on type `BenchmarkReport`
error[E0609]: no field `coordinator_page_ms` on type `BenchmarkReport`
error[E0609]: no field `coordinator_page_rows` on type `BenchmarkReport`
error[E0609]: no field `terminal_lookup_ms` on type `BenchmarkReport`
```

After implementation and the Clippy cleanup, the focused smoke test passed:

```text
cargo test -p catalog-bench --test benchmark_smoke
1 passed; 0 failed
```

The benchmark now uses deterministic paths and media kinds. Every generated
asset is shaped and belongs to `benchmark-wall`; indices divisible by 10 are
videos and all other indices are stills, giving exactly 100,000 videos and
900,000 stills for the million-asset run. The wall and coordinator pages
assert exact 100, 100, and 250 row counts and reject video rows or IDs. The
existing insert, unavailable-count, and eviction-plan measurements remain.

## Million-asset benchmark

Command:

```text
cargo run --release -p catalog-bench -- --assets 1000000 --output /tmp/photo-viewer-million-report.json
```

Report path: `/tmp/photo-viewer-million-report.json`

Report SHA-256:

```text
85c36fbc4735b8dcbde049a77cf8232cb23837dc63fe7d34cbcb4143cf8f300d
```

Report:

```json
{
  "assets": 1000000,
  "sqlite_version": "3.53.2",
  "database_bytes": 789831680,
  "insert_ms": 421234.77991700004,
  "first_page_ms": 0.259917,
  "first_page_rows": 100,
  "second_page_ms": 0.173375,
  "second_page_rows": 100,
  "coordinator_page_ms": 0.24516700000000002,
  "coordinator_page_rows": 250,
  "terminal_lookup_ms": 0.033166,
  "unavailable_count_ms": 332.129875,
  "eviction_plan_ms": 3.2902500000000003
}
```

The earlier first-page measurement was 1.08 ms at commit `3186354`. The
20-percent ceiling is 1.296 ms. The fresh 0.259917 ms result is 24.066389
percent of the earlier measurement. The benchmark uses bounded 100/100/250
page allocations and does not take a collection-sized queue snapshot.

Machine context:

```text
MacBookPro18,1, Apple M1 Pro, 10 cores, 16 GB RAM
macOS 26.5.2 (Darwin 25.5.0, arm64)
rustc 1.97.1, cargo 1.97.1
Node v24.18.0, npm 11.16.0
```

## Interface matrix

```text
npm test
Test Files  16 passed (16)
Tests       129 passed (129)

npm run test:browser
Test Files  3 passed (3)
Tests       145 passed (145)

npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion
Test Files  1 passed (1)
Tests       2 passed (2)

npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast
Test Files  1 passed (1)
Tests       2 passed (2)

npm run typecheck
exit 0

npm run check
Checked 69 files in 220ms. No fixes applied.

npm run --workspace @photo-viewer/interface build
1888 modules transformed
dist/assets/index-DJKmgDN-.css   21.67 kB, gzip 4.97 kB
dist/assets/index-CcP2hML6.js   317.79 kB, gzip 97.14 kB
```

The browser suite emitted one known non-failing React `ViewerStage` `act(...)`
warning. The motion, contrast, unit, typecheck, and Biome runs emitted no
warnings. There were no unhandled promise rejections, accessibility
violations, or screenshot attachments. The first browser attempt was blocked
by the managed sandbox's `listen EPERM` loopback restriction; the same command
passed after local-network approval.

## Rust and desktop matrix

```text
cargo test --workspace --all-features
191 passed; 0 failed; 0 ignored

cargo clippy --workspace --all-targets --all-features -- -D warnings
Finished `dev` profile; no warnings or errors

cargo fmt --all -- --check
exit 0

cargo test -p catalog-bench --test benchmark_smoke
1 passed; 0 failed

cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
11 passed; 0 failed; 0 doc-test failures

cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
Finished `dev` profile; no warnings or errors

cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check
exit 0

npm run desktop:build
Finished 1 bundle at:
/Users/jennerm/repos/photo_viewer/.worktrees/viewer-zoom-pan/apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app

git diff --check
exit 0
```

The interface assets are 317,796 bytes of JavaScript and 21,673 bytes of CSS.
The app bundle contains three files, occupies 23,176 KiB on disk, and its
native executable is 23,448,864 bytes.

## Source and cache safety

The controlled source fixture tree is
`apps/interface/public/demo-photos`. Before and after clean-profile fixture
coverage, its aggregate SHA-256 was:

```text
a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6
```

Both the per-file hash diff and the metadata diff were empty. The six files
had these stable `mtime|size` values:

```text
city.jpg|1787835664|615559
coast.jpg|1787835664|597900
forest.jpg|1787835664|865825
interior.jpg|1787835664|446647
mountain.jpg|1787835664|483651
portrait.jpg|1787835664|319646
```

The clean-profile fixture commands were:

```text
PHOTO_VIEWER_PROFILE=task-7-clean cargo test -p photo-app-service --test progressive_wall
54 passed; 0 failed

PHOTO_VIEWER_PROFILE=task-7-clean cargo test -p photo-cache --test image_derivative
10 passed; 0 failed

PHOTO_VIEWER_PROFILE=task-7-clean npm run test:browser
3 files, 145 passed; 0 failed
```

The progressive-wall suite covers scan, thumbnail and preview phase ordering,
corrupt-photo isolation, sorting-related wall requests, terminal outcomes, and
online/offline restart behavior. The cache suite covers legacy preview repair
and source-preserving derivative generation. The browser suite covers sorting,
viewer opening, and zoom. All fixture state is temporary; the checked-in
source tree stayed unchanged.

The feature-range source audit used:

```text
base=14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc
head=fd557572cd9dca2214d99f1e0b6a8d8d526794d2
```

The broad-token search returned only these contexts:

- `selectedFolderName: "Video fixture"`, a path-free browser fixture label;
- `std::fs::rename(&fixture.source, &unavailable)` and the reverse operation,
  temporary progressive-wall fixtures for source-availability simulation;
- `std::fs::remove_file(fixture.source.join("offline.jpg"))`, temporary fixture
  cleanup.

No production source-media write, delete, rename, move, or copy call was
introduced. No native source-path DTO field was introduced.

The existing wall-demo cache root was inspected without changing it:

```text
/Users/jennerm/Library/Caches/app.photoviewer.desktop/profiles/wall-demo
regular_file_count=1566
symlink_count=0
resolved_paths_outside_root=0
cache_root_exists=yes
```

## Approved behavior and concerns

- Videos remain indexed but invisible on photo surfaces until cross-platform
  playback ships.
- A wall tile opens only after its current thumbnail paints. Background work
  runs thumbnails first and previews second.
- Corrupt photos do not block healthy work.
- Escape closes Info, then resets zoom, then returns to the wall.
- The viewer remains dark under system-light appearance.
- Zoom remains cache-only. It never reads or enlarges the original source, and
  an uncached derivative has no offline recovery path.

The only non-failing automated concern is the existing `ViewerStage`
`act(...)` warning in the full browser run. Native acceptance remains
controller-owned. This task did not launch or restart the `wall-demo` app.

## Source files changed

- `crates/catalog-bench/src/lib.rs`
- `crates/catalog-bench/tests/benchmark_smoke.rs`
- `docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md`
- `README.md`
- `.superpowers/sdd/2026-08-28-derivative-coordinator-and-video-filter/task-7-report.md`
