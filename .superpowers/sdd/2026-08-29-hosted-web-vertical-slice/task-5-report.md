# Task 5 verification report

Implementation under verification: `dbfdb39` (`feat: serve managed hosted photo derivatives`), compared with `8a3de31`.

## RED / GREEN evidence

The task-5 implementation commit adds the previously absent hosted derivative API and managed-open interfaces. The pre-implementation RED expectation in `task-5-brief.md` was missing-method/route failure; this verification run was against the existing implementation and therefore focused on GREEN evidence. No implementation files were changed during verification.

Focused GREEN results:

- `photo-cache` cache policy: 15 passed; focused `open_checked`: 1 passed.
- `photo-catalog` catalog round trip: 12 passed; selection membership: 4 passed.
- `photo-app-service` hosted selections: 7 passed; source safety: 1 passed.
- `photo-server` derivative API: 1 passed.
- Adjacent progressive/offline/path-safety checks: offline reopen 1 passed; cache reuse after source removal 1 passed; managed-read checks 2 passed; path-free wall DTO 1 passed; open/reopen suite 7 passed.
- Full workspace: all unit/integration tests and doc-tests passed, including progressive wall (57), task-7 source safety (1), derivative API (1), events (10), gallery (9), folders (10), health (10), and all cache/catalog/indexer/metadata suites. No failures or ignored tests were reported.

One standalone `progressive_wall` invocation remained in `invalidation_before_commit_admission_has_no_side_effects` for about eight minutes after reporting the other 56 tests as passing and was interrupted (exit 130). The subsequent required full workspace run completed the same test and all 57 progressive-wall tests in 4.42 seconds; this appears nondeterministic timing/flakiness worth monitoring, not a reproduced functional failure.

## Exact verification commands

All Cargo commands were run serially with offline dependencies, `CARGO_BUILD_JOBS=2`, and `--jobs 2` (except no build-job flag is accepted by `cargo fmt`; the environment was still set).

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 15 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy open_checked --jobs 2
  PASS: 1 passed, 0 failed (14 filtered)
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-catalog --test catalog_round_trip --jobs 2
  PASS: 12 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-catalog --test selection_membership --jobs 2
  PASS: 4 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 7 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test task7_source_safety --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  INTERRUPTED: stalled in invalidation_before_commit_admission_has_no_side_effects; exit 130
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall offline_reopen --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall second_identical_wall_request_reuses_cache_after_the_source_goes_offline --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall read_derivative --jobs 2
  PASS: 2 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall wall_dtos --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test open_and_return --jobs 2
  PASS: 7 passed, 0 failed
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace tests and doc-tests; 0 failures
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check 8a3de31..dbfdb39
  PASS: no whitespace errors
git status --short
  PASS: clean before this report was created
```

## Diff and safety audit

`dbfdb39` changes 11 files: 698 insertions and 7 deletions. `CacheWriter::open_checked` validates relative paths, rejects traversal and symlink escapes, checks regular-file status, and opens read-only. The derivative route resolves only the opaque cache key through the catalog and never accepts a source path, range header, or original-media fallback. Source snapshot/symlink/missing-file tests pass.

Source-media mutation audit found no source-root `write`, `remove`, `rename`, or `copy` operation in the hosted derivative path. Cache writes and partial-file cleanup are confined to the canonical cache root. DTO and HTTP error paths expose opaque IDs/relative selection breadcrumbs or generic messages; no native source path is serialized. Relevant path-free tests passed, including `wall_dtos_never_serialize_native_paths`, `bootstrap_does_not_expose_a_native_source_path`, and the task-7 source snapshot harness.

## Concern (resolved in fix round 1)

`crates/server/src/api/derivative.rs` calls `Read::read_to_end` and constructs `Body::from(bytes)`, so delivery is safely cache-scoped but currently buffers the complete derivative rather than streaming the managed file. If “managed streaming” is a hard requirement for large derivatives, this should be addressed in a follow-up.

## Task 5 fix round 1

Implementation/fix commit: `3aa171a` (`fix: harden hosted derivative generation and delivery`), based on implementation `dbfdb39`.

The first regression test was deliberately run before the fix:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections corrupt_hosted_thumbnail_is_repaired_before_it_is_reused --jobs 2
  RED: failed; corrupt cache bytes were returned instead of a regenerated JPEG
```

The fix replaces the hosted per-key lock path with coordinator-backed work tickets and commit admission, preserves protected-group guards through generation/commit, rechecks current asset signature and cache key before commit, validates IDs and scope before scan admission, repairs stale regular cache records, verifies opened descriptors, and streams managed files in 64 KiB async chunks. `If-None-Match` now accepts wildcard, list, and weak matches. Changed files are `crates/app-service/src/gallery.rs`, `crates/app-service/tests/hosted_selections.rs`, `crates/cache/Cargo.toml`, `crates/cache/src/writer.rs`, `crates/server/src/api/derivative.rs`, and `Cargo.lock`.

Fix-round GREEN evidence:

- `photo-cache` cache policy: 15 passed; `open_checked`: 1 passed.
- `photo-app-service` hosted selections, including corrupt size/signature and concurrent repair: 8 passed; progressive wall: 57 passed; task-7 source safety: 1 passed.
- `photo-server` derivative API: 1 passed; server library tests: 2 passed (including ETag matching).
- The invalidation test was reproduced alone with a 45-second outer timeout and then run three serial times; each passed (exit 0, approximately 0.47–0.63 seconds). It was not reproduced as a hang.
- Required full workspace command passed all unit/integration tests and doc-tests with zero failures.
- Formatting and whitespace checks passed.

Exact fix-round commands/results:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy open_checked --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 15 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 8 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib --jobs 2
  PASS: 2 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  PASS: 57 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test task7_source_safety --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 perl -e '$SIG{ALRM}=sub { exit 124 }; alarm 45; exec @ARGV' cargo test --offline -p photo-app-service --test progressive_wall invalidation_before_commit_admission_has_no_side_effects --jobs 2
  PASS: 1 passed, 0 failed (repeated three times serially)
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace tests and doc-tests; 0 failures
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check
  PASS: no whitespace errors
```

All Cargo commands were run one at a time, offline, with `CARGO_BUILD_JOBS=2` and `--jobs 2`; no background build was launched and existing target artifacts were reused. The source-media audit found no hosted derivative write/delete/rename/copy operations under source roots. Cache mutation remains managed-cache-only. The open-check implementation uses descriptor-relative opening/containment on Unix/macOS and a Windows no-follow final-handle plus descriptor-containment check; no deterministic swap-race harness was added because no safe cross-platform deterministic scheduling hook exists in the current cache test fixtures.

## Task 5 fix round 2

Implementation/fix commit: `2b4c11e` (`fix: close hosted derivative coordination races`), based on `3aa171a`.

Root-cause RED evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections corrupt_hosted_thumbnail_is_repaired_before_it_is_reused --jobs 2
  RED: with a live subscription retained, repair returned Err(NotFound); the coordinator replayed its remembered completion after the stale cache row was removed and no worker ran.
```

The fix invalidates remembered completions when a catalogued cache record is stale, carries requested scope in hosted work keys, rechecks many-to-many selection membership before and after encoding, encodes wall thumbnails before the current-key fence and commits afterward, bounds hosted attempts with shared admission, runs one supervised driver per selection runtime (without engine-wide unfair draining), and restarts queued work after a driver panic. Unix/macOS cache reads and deletes now walk opened no-follow directory descriptors; FIFOs are opened nonblocking and rejected before media use. Windows uses a no-follow final handle and validates its final descriptor before use/deletion. Typed derivative failures now map to explicit HTTP error codes. Added hosted scope/foreign, retained-runtime repair, offline route, route-size guard, FIFO, and safe-delete regressions.

Fix-round GREEN evidence:

- `photo-cache` cache policy: 17 passed, including FIFO rejection and symlinked-ancestor deletion safety.
- `photo-app-service` library/coordinator tests: 92 passed; hosted selections: 9 passed, including current-folder/foreign membership and live-runtime repair; hosted runtime: 15 passed.
- `photo-server` derivative API: 2 passed, including offline cache delivery and decoded-ID/query-size guards.
- Required full workspace command: all unit/integration tests and doc-tests passed, including progressive wall (57), source safety (1), catalog, indexer, metadata, and server suites.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

Exact round-2 commands/results:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections corrupt_hosted_thumbnail_is_repaired_before_it_is_reused --jobs 2
  RED: stale retained completion caused NotFound after repair
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 17 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 9 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 2 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib --jobs 2
  PASS: 92 passed, 0 failed
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace tests and doc-tests; 0 failures
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check
  PASS: no whitespace errors
```

Round-2 concern: Unix/macOS uses descriptor-relative no-follow component walks for opening and unlinking. Windows avoids following the final reparse point and checks the opened handle’s final path before reading/deleting, but Windows ancestor traversal remains a platform boundary because the standard library has no handle-relative `CreateFile` equivalent; this is explicitly not claimed as descriptor-relative Windows ancestor opening. No Cargo process was launched in the background; the final full gate exited successfully before report generation.
