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
The lifecycle correction is committed in
`27af6bc955d0d147ee6db25d88fe2b9bf80e216b` (`test: close app-service lifecycle
before restart`). The round-3 collection-driver admission correction is
committed in `d8a01019a5eef89a689bed0d1c6b213fbed7460e` (`test: admit
collection drivers before spawn`). The verification/report follow-up is
intentionally separate; its post-commit name-status comparison is documented
below so the later commit is not mistaken for an implementation change or
given an impossible self-referential SHA.

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

The delayed collection-driver regression was then added before moving the
tracker admission ahead of `tokio::spawn`. Against the late-admission version,
the new test was red because the driver had not yet been polled:

```text
cargo test -p photo-app-service collection_driver_is_tracked_before_its_first_poll -- --nocapture
assertion `left == right` failed: driver admission must be visible before the spawned future is first polled
  left: 0
 right: 1
```

The implementation now acquires the test/debug tracking guard synchronously,
moves it into the spawned future, and lets guard drop cover normal completion,
cancellation, panic, or spawn unwind. The test deliberately exercises the
driver's initial delay and asserts that quiescence cannot complete while the
not-yet-polled driver is admitted:

```text
cargo test -p photo-app-service collection_driver_is_tracked_before_its_first_poll -- --nocapture
test derivatives::tests::collection_driver_is_tracked_before_its_first_poll ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 63 filtered out; finished in 0.11s
```

The focused admission regression passed 100 consecutive repetitions. The
complete source-safety/restart harness passed 10 consecutive repetitions; each
full run includes the read-only source snapshot and real service restart and
took about 9.7 seconds, so 100 complete repetitions were not practical in the
available run window:

```text
collection_driver_first_poll_repetitions=100
source_safety_restart_repetitions=10
```

The lifecycle regression was then exercised test-first. Adding the collection
driver probe before its implementation produced the expected compile failure:

```text
cargo test -p photo-app-service --test task7_source_safety -- --nocapture
error[E0599]: no method named `collection_driver_active_count_test` found for struct `AppService`
```

After the probe and tracker were added, the old single-scope restart boundary
was forced to fail before the scope fix. The tracked collection-driver count
was four while interaction remained Active even though derivative work had
reported zero:

```text
thread 'task7_source_tree_safety_harness' panicked at crates/app-service/tests/task7_source_safety.rs:256:5:
assertion `left == right` failed: the first service scope must not overlap a restart
  left: 4
  right: 0
```

The corrected test transitions the first service to Idle, waits for both
derivative and collection-driver trackers, asserts zero active scan/driver/
derivative counters, drops update receivers, exits that scope, and only then
opens the restarted service. The focused GREEN result after the correction was:

```text
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
270 passed; 0 failed; 0 ignored

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
the same temporary root, and keeps every catalogue/cache handle within that
temporary root.

The fix-round capture artifacts were
`/tmp/photo-viewer-task7-round2-before-mtime.txt`,
`/tmp/photo-viewer-task7-round2-after-mtime.txt`,
`/tmp/photo-viewer-task7-round2-before-file-sha.txt`, and
`/tmp/photo-viewer-task7-round2-after-file-sha.txt`; the recorded comparison
artifacts `/tmp/photo-viewer-task7-round2-mtime.diff`,
`/tmp/photo-viewer-task7-round2-file-sha.diff`, and
`/tmp/photo-viewer-task7-round2-source-status.txt` were all zero lines.

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

The lifecycle boundary is explicit rather than inferred from dropping a
service handle. With interaction Active, the old sequence recorded four live
collection drivers after the derivative tracker reached zero. The test-only
collection-driver tracker now covers each spawned driver, including its delay
and collection mutex wait. Admission is recorded synchronously before the
spawned future's first poll, so the delayed-driver regression cannot observe a
false zero. The first service transitions to Idle, awaits zero
derivative tasks and zero collection drivers, asserts zero active scan/driver/
derivative counters, drops both update receivers, and exits its lexical scope.
Only in the next scope does the test open the restarted service and issue the
offline cached request; it awaits both trackers and asserts all counters are
zero before that scope ends. No sleep, runtime-drop ordering, or environment
profile is used as a quiescence signal.

The browser tests use the same `/demo-photos/*.jpg` URLs for wall sorting,
viewer opening, and zoom. The full browser run emitted one known non-failing
React `ViewerStage` `act(...)` warning; there was no unhandled rejection,
accessibility violation, or screenshot attachment. The progressive-wall suite
and cache suite remain temporary-fixture-only; no desktop process was started.

The source audit was rerun from the accepted base through the complete code/test
head in lifecycle implementation commit
`d8a01019a5eef89a689bed0d1c6b213fbed7460e`:

```text
base=14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc
head=d8a01019a5eef89a689bed0d1c6b213fbed7460e
```

The exact added-line audit command scans Rust, TypeScript, and TSX for broad
filesystem create/write/save/copy/move/rename/remove/delete/unlink forms,
`OpenOptions`, `File::create`, `write_all`, `write_atomic`, rating/tag/delete
operations, source/cache/folder/path DTO tokens, `relative_cache_path`, and
`write_png`:

```text
base=14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc
head=d8a01019a5eef89a689bed0d1c6b213fbed7460e
git diff --name-status "$base..$head" -- '*.rs' '*.ts' '*.tsx'
git diff --unified=0 --no-color "$base..$head" -- '*.rs' '*.ts' '*.tsx' | awk '
/^\+\+\+ b\// { file=substr($0,7); next }
/^@@ / { p=index($0,"+"); if (p) { h=substr($0,p+1); sub(",.*","",h); line=h+0 }; next }
/^\+/ && !/^\+\+\+/ {
  text=substr($0,2)
  if (text ~ /(std::fs::(create_dir_all|write|rename|remove_file|remove_dir_all|copy)|fs::(write|rename|remove|copy)|File::create|OpenOptions|write_all|write_atomic|write_png|writeFile|write_file|copyFile|copy_file|moveFile|move_file|rename|removeFile|remove_file|unlink|delete_derivatives|deleteAsset|setRating|updateRating|\.save[[:space:]]*\(|selectedFolder(Name)?|selected_folder|sourcePath|folderPath|relativePath|relative_cache_path|displayPath|nativePath|cachePath|locateFolder)/) {
    sub(/^[[:space:]]+/, "", text)
    print file ":" line ": " text
  }
  line++
}
'
```

The command's direct output contained 45 name-status lines and 56 matching
added lines. The captured direct outputs are
`/tmp/photo-viewer-task7-audit-round3-name-status.txt` (SHA-256
`65815f310b627be306187c4c09cc2fee9df2ba2863625fc0e049cb69f46cd767`) and
`/tmp/photo-viewer-task7-audit-round3-matches.txt` (SHA-256
`e1c5e509255c759b827d2c9317b41d9e41f47368a7cb1b569d4dcccee0e1d135`). Every
match from that direct output is reproduced below with its file and line; the
classification after the list covers each match.

```text
apps/interface/src/components/PhotoViewer.browser.test.tsx:1933: selectedFolderName: "Video fixture",
crates/app-service/src/derivatives.rs:419: .map(|record| record.relative_cache_path.clone())
crates/app-service/src/derivatives.rs:987: relative_cache_path: generated.relative_path.clone(),
crates/app-service/src/derivatives.rs:1212: .map(|record| record.relative_cache_path)
crates/app-service/src/derivatives.rs:1253: .map(|record| record.relative_cache_path)
crates/app-service/src/derivatives.rs:2937: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:2941: .save(&blocked_path)
crates/app-service/src/derivatives.rs:2944: .save(&healthy_path)
crates/app-service/src/derivatives.rs:3023: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:3025: .save(source.join("photo.jpg"))
crates/app-service/src/derivatives.rs:3117: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:3120: .save(&image_path)
crates/app-service/src/derivatives.rs:3173: relative_cache_path: std::path::PathBuf::from(format!(
crates/app-service/src/derivatives.rs:3344: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:3347: .save(&image_path)
crates/app-service/src/derivatives.rs:3417: std::fs::create_dir_all(&source).unwrap();
crates/app-service/src/derivatives.rs:3420: .save(&image_path)
crates/app-service/src/derivatives.rs:3520: .save(&source_path)
crates/app-service/src/derivatives.rs:3691: std::fs::create_dir_all(&source).unwrap();
crates/app-service/tests/progressive_wall.rs:140: relative_cache_path: PathBuf,
crates/app-service/tests/progressive_wall.rs:188: relative_cache_path: record.relative_cache_path,
crates/app-service/tests/progressive_wall.rs:222: let relative_cache_path = PathBuf::from("evictable/old-preview.jpg");
crates/app-service/tests/progressive_wall.rs:223: std::fs::create_dir_all(
crates/app-service/tests/progressive_wall.rs:227: .join(relative_cache_path.parent().unwrap()),
crates/app-service/tests/progressive_wall.rs:230: std::fs::write(
crates/app-service/tests/progressive_wall.rs:231: fixture.config.cache_dir().join(&relative_cache_path),
crates/app-service/tests/progressive_wall.rs:242: relative_cache_path,
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
crates/app-service/tests/progressive_wall.rs:2741: .join(&screen_row.relative_cache_path)
crates/app-service/tests/progressive_wall.rs:3829: relative_cache_path: PathBuf::from("video-placeholder.webp"),
crates/app-service/tests/progressive_wall.rs:3993: std::fs::remove_file(fixture.source.join("offline.jpg")).unwrap();
crates/app-service/tests/task7_source_safety.rs:289: assert_eq!(catalog.delete_derivatives(&[wall.id]).unwrap(), 1);
crates/cache/src/image_derivative.rs:141: let write_result = self.writer.write_atomic(relative_path.clone(), |file| {
crates/cache/src/image_derivative.rs:142: std::io::Write::write_all(file, &encoded)
crates/cache/tests/image_derivative.rs:81: std::fs::copy(fixture.path(), &cached_screen).unwrap();
crates/indexer/tests/progressive_scan.rs:324: write_png(&fixture.path().join("a.jpg"), [255, 0, 0]);
crates/indexer/tests/progressive_scan.rs:325: write_png(&fixture.path().join("b.jpg"), [0, 0, 255]);
crates/indexer/tests/progressive_scan.rs:326: std::fs::write(fixture.path().join("clip.mp4"), b"not a video").unwrap();
```

Classification of all 56 matches:

- The browser `selectedFolderName` value is a path-free fixture label.
- The four production `derivatives.rs` matches at lines 419, 987, 1212, and
  1253 only read or record managed-cache-relative catalogue metadata; they do
  not perform filesystem operations. The `derivatives.rs` matches at lines
  2937--3691 and the `relative_cache_path` line at 3173 are inside its test
  module and create temporary image fixtures or temporary catalogue/cache
  records.
- The `progressive_wall.rs` matches at lines 140, 188, 222, 227, 230, 231,
  and 242 describe or create an evictable path under the test's temporary
  cache directory. Lines 355--379 and 1607--1967 create temporary image/video
  fixtures or write changed-signature test input. The two `rename` lines at
  1489/1521 move only a temporary fixture to simulate source availability.
  The `delete_derivatives` line at 2238 operates on temporary catalogue
  records; line 2741 inspects a path under the temporary cache root; line 3829
  records a temporary catalogue placeholder; and line 3993 removes a
  temporary fixture.
- The safety test's `delete_derivatives` line at 289 removes only its
  temporary catalogue row while leaving the checked-in source untouched.
- `CacheWriter::write_atomic` and `Write::write_all` in production
  `crates/cache/src/image_derivative.rs` write managed cache files. The cache
  test copy and the indexer `write_png` calls at lines 324/325 plus the video
  fixture write at 326 operate under temporary fixtures.

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

The implementation/test boundary is the round-3 lifecycle code/test commit
`d8a01019a5eef89a689bed0d1c6b213fbed7460e`. The subsequent verification and
report changes are a separate documentation-only follow-up. After that
follow-up commit, the exact name/status comparison was:

```text
git diff --name-status d8a01019a5eef89a689bed0d1c6b213fbed7460e..HEAD
M	.superpowers/sdd/2026-08-28-derivative-coordinator-and-video-filter/task-7-report.md
M	docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md
```

This report deliberately records the implementation SHA and the docs-only
comparison, while the final documentation commit SHA is reported by the
post-commit `git show` handoff rather than embedded in its own content.

## Final whole-branch collection-driver correction (2026-08-28)

The final collection-driver implementation/test head is
`aa7a268e2f7fc86d27d682d94f54b049dfdf6780` (`fix: coalesce collection driver
wakeups`). It replaces one delayed task per interaction/request with a single
service-owned coalescing admission, keeps recent intent in the bounded
coordinator window, merges full-group intent monotonically, and proves the
exit handoff cannot lose a request.

The Task 7 restart/source-safety harness was rerun against this head as part
of the final workspace run and passed. The direct harness result was:

```text
cargo test -p photo-app-service --test task7_source_safety -- --test-threads=1
test result: ok; 1 passed; 0 failed
```

The controlled demo source tree still has six regular files and identical
before/after size, mtime, and content-digest snapshots. The final source
audit from `6d4296b52429d8aa5806492d3e25c8820efad02d` through
`aa7a268e2f7fc86d27d682d94f54b049dfdf6780` contains two Rust name-status
lines and zero added-line filesystem/source-path matches. The direct-output
hashes are `4d1900107842ab4cd6e1c0b4fbb28404c0ff07df4cae4f73917df7357583ece9`
for name-status and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` for the
zero-match audit. The before/after source-safety snapshot hash is
`7d44d5aa0366d1e2dc63d09486637b3790db056e3fe071edc9cec8055c96c979`.

## Source files changed

- `Cargo.lock`
- `crates/app-service/Cargo.toml`
- `crates/app-service/src/derivatives.rs`
- `crates/app-service/src/service.rs`
- `crates/app-service/tests/task7_source_safety.rs`
- `crates/catalog-bench/src/lib.rs`
- `crates/catalog-bench/tests/benchmark_smoke.rs`
- `docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md`
- `README.md`
- `.superpowers/sdd/2026-08-28-derivative-coordinator-and-video-filter/task-7-report.md`
