# Task 7 report: coordinated photo-loading verification

Date: 2026-08-28
Branch: `codex/viewer-zoom-pan`

## Scope

Task 7 adds automated benchmark coverage and records the verification evidence
for the coordinator, photo-only projections, viewer, and desktop bundle. No
desktop development process was started. No merge, push, worktree deletion, or
native user-acceptance action was performed.

This fix round adds one test-only app-service harness for the exact checked-in
demo source tree. The harness and its test-only BLAKE3 dependency are committed
in `67393e1224f8cf6e6944e8fd4a625ff0baf6bbe5` (`test: guard demo source tree`).
The verification/report follow-up is intentionally separate; its post-commit
name-status comparison is documented below so the later commit is not mistaken
for an implementation change or given an impossible self-referential SHA.

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

The safety harness was then written test-first. Its first compile run was
red because the test called the not-yet-defined sequence helper:

```text
cargo test -p photo-app-service --test task7_source_safety
error[E0425]: cannot find function `run_task7_source_safety_sequence` in this scope
```

After the test-only harness was implemented, both formatting and the focused
test were green:

```text
cargo fmt --all -- --check
exit 0

cargo test -p photo-app-service --test task7_source_safety -- --nocapture
test task7_source_tree_safety_harness ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
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
192 passed; 0 failed; 0 ignored

cargo clippy --workspace --all-targets --all-features -- -D warnings
Finished `dev` profile; no warnings or errors

cargo fmt --all -- --check
exit 0

cargo test -p catalog-bench --test benchmark_smoke
1 passed; 0 failed

cargo test -p photo-app-service --test task7_source_safety -- --nocapture
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
`apps/interface/public/demo-photos`. One safety window captured its complete
file manifest immediately before the app-service and browser operations, then
captured it again after every process exited. The six-file `mtime|size`
manifest was identical at both boundaries:

```text
apps/interface/public/demo-photos/city.jpg|1787835664|615559
apps/interface/public/demo-photos/coast.jpg|1787835664|597900
apps/interface/public/demo-photos/forest.jpg|1787835664|865825
apps/interface/public/demo-photos/interior.jpg|1787835664|446647
apps/interface/public/demo-photos/mountain.jpg|1787835664|483651
apps/interface/public/demo-photos/portrait.jpg|1787835664|319646
```

The exact command window was:

```text
find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 stat -f '%N|%m|%z'
find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256

cargo test -p photo-app-service --test task7_source_safety -- --nocapture
test task7_source_tree_safety_harness ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

npm run test:browser
Test Files  3 passed (3)
Tests       145 passed (145)

find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 stat -f '%N|%m|%z'
find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256
```

Both aggregate SHA-256 outputs were
`a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6`.
The metadata diff and per-file hash diff each had 0 lines, and
`git status --short -- apps/interface/public/demo-photos` had no output. No
`PHOTO_VIEWER_PROFILE` variable was used. The harness creates a
`tempdir()/catalog` and `tempdir()/cache`, asserts both paths are contained by
the same temporary root, and drops the restarted service before the after
snapshot.

The committed test-only harness opens that exact tree read-only and performs
the whole app-service sequence: scan; first and cursor-continuation wall pages;
oldest/newest sorts; current visible and near-viewport neighbour request
planning; a complete wall-thumbnail phase followed by a complete screen-preview
phase; a legacy screen-cache-to-wall repair after deleting only a temporary
catalogue row and marking the catalogue root offline; a service restart outside
the runtime; and an offline cached preview request. It asserts six exact JPEG
rows and no video rows throughout. The harness snapshots every source file's
bytes, BLAKE3 digest, size, and nanosecond mtime before the first service open
and after the restarted service is dropped.

The browser tests use the same `/demo-photos/*.jpg` URLs for wall sorting,
viewer opening, and zoom. The full browser run emitted one known non-failing
React `ViewerStage` `act(...)` warning; there was no unhandled rejection,
accessibility violation, or screenshot attachment. The progressive-wall suite
and cache suite remain temporary-fixture-only; no desktop process was started.

The source audit was rerun from the accepted base through the complete code/test
head in implementation commit `67393e1224f8cf6e6944e8fd4a625ff0baf6bbe5`:

```text
base=14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc
head=67393e1224f8cf6e6944e8fd4a625ff0baf6bbe5
```

The exact added-line audit command scans Rust, TypeScript, and TSX for broad
filesystem create/write/save/copy/move/rename/remove/delete/unlink forms,
`OpenOptions`, `File::create`, `write_all`, `write_atomic`, rating/tag/delete
operations, and source/cache/folder/path DTO tokens:

```text
git diff --name-status "$base..$head" -- '*.rs' '*.ts' '*.tsx'
git diff --unified=0 --no-color "$base..$head" -- '*.rs' '*.ts' '*.tsx' | awk '
/^\+\+\+ b\// { file=substr($0,7); next }
/^@@ / { p=index($0,"+"); if (p) { h=substr($0,p+1); sub(",.*","",h); line=h+0 }; next }
/^\+/ && !/^\+\+\+/ {
  text=substr($0,2)
  if (text ~ /(std::fs::(create_dir_all|write|rename|remove_file|remove_dir_all|copy)|fs::(write|rename|remove|copy)|File::create|OpenOptions|write_all|write_atomic|writeFile|write_file|copyFile|copy_file|moveFile|move_file|rename|removeFile|remove_file|unlink|delete_derivatives|deleteAsset|setRating|updateRating|\.save[[:space:]]*\(|selectedFolder(Name)?|selected_folder|sourcePath|folderPath|relativePath|displayPath|nativePath|cachePath|locateFolder)/) print file ":" line ": " text
  line++
}
'
```

The added-line audit returned 41 matches. Every match is reproduced below with
its file and line; the classification after the list covers each match.

```text
apps/interface/src/components/PhotoViewer.browser.test.tsx:1933: selectedFolderName: "Video fixture"
crates/app-service/src/derivatives.rs:2910: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:2914: .save(&blocked_path)
crates/app-service/src/derivatives.rs:2917: .save(&healthy_path)
crates/app-service/src/derivatives.rs:2996: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:2998: .save(source.join("photo.jpg"))
crates/app-service/src/derivatives.rs:3090: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:3093: .save(&image_path)
crates/app-service/src/derivatives.rs:3289: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:3292: .save(&image_path)
crates/app-service/src/derivatives.rs:3362: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:3365: .save(&image_path)
crates/app-service/src/derivatives.rs:3465: .save(&source_path)
crates/app-service/src/derivatives.rs:3636: std::fs::create_dir_all(&source).unwrap();
crates/app-service/tests/progressive_wall.rs:223: std::fs::create_dir_all(
crates/app-service/tests/progressive_wall.rs:227: .join(relative_cache_path.parent().unwrap()),
crates/app-service/tests/progressive_wall.rs:230: std::fs::write(
crates/app-service/tests/progressive_wall.rs:355: std::fs::create_dir_all(&source).unwrap();
crates/app-service/tests/progressive_wall.rs:356: std::fs::write(source.join("clip.mp4"), b"not a video").unwrap();
crates/app-service/tests/progressive_wall.rs:368: std::fs::create_dir_all(&source).unwrap();
crates/app-service/tests/progressive_wall.rs:370: .save(source.join("a.jpg"))
crates/app-service/tests/progressive_wall.rs:372: std::fs::write(source.join("corrupt.jpg"), b"not a jpeg").unwrap();
crates/app-service/tests/progressive_wall.rs:374: .save(source.join("offline.jpg"))
crates/app-service/tests/progressive_wall.rs:377: .save(source.join("b.jpg"))
crates/app-service/tests/progressive_wall.rs:379: std::fs::write(source.join("clip.mp4"), b"not a video").unwrap();
crates/app-service/tests/progressive_wall.rs:1489: std::fs::rename(&fixture.source, &unavailable).unwrap();
crates/app-service/tests/progressive_wall.rs:1521: std::fs::rename(unavailable, &fixture.source).unwrap();
crates/app-service/tests/progressive_wall.rs:1607: std::fs::create_dir_all(&source_b).unwrap();
crates/app-service/tests/progressive_wall.rs:1609: .save(source_b.join("replacement.jpg"))
crates/app-service/tests/progressive_wall.rs:1678: std::fs::write(
crates/app-service/tests/progressive_wall.rs:1729: std::fs::write(fixture.source.join("photo-000.jpg"), changed).unwrap();
crates/app-service/tests/progressive_wall.rs:1789: std::fs::write(
crates/app-service/tests/progressive_wall.rs:1825: std::fs::write(fixture.source.join("photo-000.jpg"), changed).unwrap();
crates/app-service/tests/progressive_wall.rs:1868: std::fs::write(
crates/app-service/tests/progressive_wall.rs:1967: std::fs::write(fixture.source.join("photo-000.jpg"), changed).unwrap();
crates/app-service/tests/progressive_wall.rs:2238: catalog.delete_derivatives(&screen_ids).unwrap();
crates/app-service/tests/progressive_wall.rs:3993: std::fs::remove_file(fixture.source.join("offline.jpg")).unwrap();
crates/app-service/tests/task7_source_safety.rs:279: assert_eq!(catalog.delete_derivatives(&[wall.id]).unwrap(), 1);
crates/cache/src/image_derivative.rs:141: let write_result = self.writer.write_atomic(relative_path.clone(), |file| {
crates/cache/src/image_derivative.rs:142: std::io::Write::write_all(file, &encoded)
crates/cache/tests/image_derivative.rs:81: std::fs::copy(fixture.path(), &cached_screen).unwrap();
crates/indexer/tests/progressive_scan.rs:326: std::fs::write(fixture.path().join("clip.mp4"), b"not a video").unwrap();
```

Classification of all 41 matches:

- The browser `selectedFolderName` value is a path-free fixture label.
- Every `derivatives.rs` match at lines 2910--3636 is inside the test module
  and creates temporary image fixtures or records temporary cache paths.
- Every `progressive_wall.rs` create/save/write match at lines 222--379,
  1607--1868, and 1967 is temporary fixture setup or changed-signature test
  input. The two `rename` lines at 1489/1521 move only a temporary fixture to
  simulate source availability. The `delete_derivatives` line at 2238
  operates on temporary catalogue records. The `remove_file` line at 3993
  removes a temporary fixture.
- The new safety test's `delete_derivatives` line at 279 removes only its
  temporary catalogue row while leaving the checked-in source untouched.
- `CacheWriter::write_atomic` and `Write::write_all` in production
  `crates/cache/src/image_derivative.rs` write managed cache files. The cache
  test copy and indexer test write operate under temporary fixtures.

No production source-media write, delete, rename, move, or copy call was
introduced. The changed app-service DTO has no native source-path field; the
separate DTO-token review found only the path-free fixture label above.

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

## Commit boundary evidence

The implementation/test boundary is the code commit
`67393e1224f8cf6e6944e8fd4a625ff0baf6bbe5`. The subsequent verification,
README, and report changes are a separate documentation-only follow-up. After
that follow-up commit, the exact name/status comparison was:

```text
git diff --name-status 67393e1224f8cf6e6944e8fd4a625ff0baf6bbe5..HEAD
M	.superpowers/sdd/2026-08-28-derivative-coordinator-and-video-filter/task-7-report.md
M	README.md
M	docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md
```

This report deliberately records the implementation SHA and the docs-only
comparison, while the final documentation commit SHA is reported by the
post-commit `git show` handoff rather than embedded in its own content.

## Source files changed

- `Cargo.lock`
- `crates/app-service/Cargo.toml`
- `crates/app-service/tests/task7_source_safety.rs`
- `crates/catalog-bench/src/lib.rs`
- `crates/catalog-bench/tests/benchmark_smoke.rs`
- `docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md`
- `README.md`
- `.superpowers/sdd/2026-08-28-derivative-coordinator-and-video-filter/task-7-report.md`
