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

## Task 5 fix round 3

Implementation/fix commit: `27b46b0` (`fix: preserve derivative priority across hosted selections`), based on `2b4c11e`.

Root-cause RED evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections corrupt_hosted_thumbnail_is_repaired_before_it_is_reused --jobs 2
  RED: with a live subscription retained, repair returned Err(NotFound); the coordinator replayed a retained completion even though the stale cache row had been removed.
```

Round-3 changes make the immutable derivative identity independent of waiter scope while retaining scope for admission and event filtering. Hosted drivers now use an engine-wide priority-aware scheduler family reservation, shared bounded admission, atomic empty-to-idle handoff, and supervised restart/abort cleanup. The retained-completion repair path invalidates the coordinator result before re-enqueue. A direct scheduler regression proves a later visible job from another selection is reserved before queued background work. Existing Unix/macOS descriptor-relative no-follow cache opening and deletion, Windows final-handle validation, managed-file streaming, source-snapshot checks, and typed HTTP ETag/size guards remain covered by the prior round.

Exact round-3 GREEN commands/results:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections corrupt_hosted_thumbnail_is_repaired_before_it_is_reused --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy open_checked --jobs 2
  PASS: 2 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 17 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_derivative_requests_enforce_current_folder_and_foreign_membership --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 9 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 2 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib --jobs 2
  PASS: 92 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-indexer --test scheduler_priority family_owned_dequeue_preserves_priority_across_selection_drivers --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences (after cargo fmt --all)
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace tests and doc-tests; 0 failures
git diff --check
  PASS: no whitespace errors
```

The final full workspace output included hosted runtime (15), hosted selections (9), progressive wall (57), task-7 source safety (1), cache policy (17), server derivative API (2), the new scheduler priority test (12 total), and all remaining workspace tests/doc-tests. No background Cargo/rustc process was launched; the sole final full command exited 0 before this report was written.

## Round-3 audit concerns

The scheduler family reservation closes the cross-selection dequeue race, but the bounded admission check is still cooperative: work already encoding cannot be preempted when a higher-priority request arrives. The post-encode stale-key cleanup prevents reusable stale cache/catalog state after the final check, but a scan mutation racing exactly between that check and publication is not protected by one shared catalog/cache admission lock. Hosted worker failures still settle through the existing `Option<DerivativeReference>` coordinator channel, so an underlying encode/join/cache cause is not preserved distinctly as `DerivativeFailed`; route mapping exists, but the hosted path currently reports unavailable for that class of failure. Windows ancestor traversal remains the documented platform boundary described above. These are explicit residual concerns, not claims of complete linearized publication or cause-preserving settlement.

Source-media audit: hosted derivative writes, deletes, and cleanup target only managed cache paths; source roots are read-only. Browser-visible derivative responses use opaque cache keys and path-free error envelopes. No Cargo/rustc process was left running after the final gate; process enumeration is restricted on this host, so this statement is based on the foreground command's exit and the absence of any background launch.

## Task 5 fix round 4

Implementation/test commit: `19abd4a` (`fix: harden hosted derivative supervision and publication`), based on `a3fce15`.

Round-4 RED evidence was collected before the coordinator settlement implementation:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib coalesced_waiters_are_filtered_by_their_own_scope_at_settlement --jobs 2
  RED: compile failed because the new direct hosted coordinator test referenced the not-yet-implemented complete_commit_filtered method
```

The fix keeps immutable work identity independent of scope but stores each waiter’s requested `GalleryScope`, filters commit settlement per waiter, and retains runtime subscription filtering for events. Hosted driver ownership is now a coordinator-locked generation token rather than a runtime boolean. One durable supervisor reserves a bounded slot before removing work from the global scheduler family, waits on coordinator/scheduler/admission generations, and catches attempt panics so queued work and all waiters are settled deterministically. The scheduler’s family reservation and global priority hint preserve visible > near-viewport > background ordering across hosted selections without placing a dequeued low-priority job behind a FIFO semaphore.

Encoded bytes remain private until a final current-key/membership fence. The hosted publication fence is shared with hosted scan batch, generation completion, source-unavailable, and existing-record catalog writes. A deterministic direct hosted test changes the catalog signature at the fence and verifies `DerivativeUnavailable`, no derivative row, and no publication. Stale-record repair removes a requester’s link before considering physical deletion and leaves shared immutable rows/files linked to another group intact. Encode, join, cache, and catalog failures record a failure outcome and reach the request as `DerivativeFailed`; membership/cancellation/offline gaps remain `DerivativeUnavailable`. Non-ASCII decoded route IDs are rejected as `invalidRequest` before catalog access.

Exact fix-round commands/results (the required workspace gate was run once after the initial focused gates; the final publication-fence tightening was then checked with focused hosted gates):

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-indexer --test scheduler_priority --jobs 2
  PASS: 12 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib --jobs 2
  PASS: 95 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 10 passed, 0 failed (before final fence test)
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 2 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 17 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  PASS: 57 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test task7_source_safety --jobs 2
  PASS: 1 passed, 0 failed
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace tests and doc-tests; 0 failures
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_encode_failure_reaches_the_request_as_derivative_failed --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections corrupt_hosted_thumbnail_is_repaired_before_it_is_reused --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_membership_fence_rejects_staged_bytes_before_publication --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 11 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_runtime --jobs 2
  PASS: 15 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall admitted_preview_panic_fails_waiters_and_releases_invalidation --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo fmt --all
  PASS: formatting completed
git diff --check
  PASS: no whitespace errors
```

All Cargo commands were foreground, serial, offline, and used `CARGO_BUILD_JOBS=2` plus `--jobs 2`; no `cargo clean` or background build was used. The full workspace gate exited successfully before the final focused fence-only review. The final focused hosted tests exited successfully afterward. Process enumeration is restricted on this host (`pgrep` cannot access the process table); every foreground Cargo command returned, and no rustc child was intentionally left behind.

## Round-4 evidence limits

The generation fence and deterministic test establish rejection before publication for the exercised in-process and direct-catalog mutation window; they do not claim a universal transaction covering arbitrary external processes after the final recheck. The durable owner and attempt join handling are covered by coordinator owner tests and the hosted panic regression, but an OS-level crash is outside the process contract. Existing cache containment coverage remains Unix/macOS descriptor-relative plus Windows final-handle validation; no new Windows ancestor traversal guarantee is claimed. Existing source snapshot and HTTP path/error tests remain covered by the workspace gate; this round adds no claim beyond those passing tests.

## Task 5 fix round 5

Implementation/test commit: `cce983d` (`fix: close final hosted derivative races`), based on `bcfe0c0`.

Round-5 RED evidence was a behavioral hosted HTTP regression, not a compile-only probe:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  RED: derivative_request_exposes_invalid_unavailable_and_failed_envelopes failed; missing-source request returned HTTP 500 instead of the expected derivativeUnavailable HTTP 503
```

The final fix classifies a missing/non-regular source as `DerivativeUnavailable` before encoding. It also carries typed `DerivativeResult::{Ready,Failed,Unavailable}` through every hosted waiter and completion entry; late waiters validate their own scope at arrival. A dropped/cancelled/panicking hosted supervisor settles its active ticket as failure, releases ownership, and starts a successor for queued work. Hosted drivers serialize bounded admission behind one shared owner and select only the best queued job in the hosted derivative family, so unrelated index work cannot block it.

Publication now holds the shared in-process hosted publication fence across current-key/signature/membership checks, cache publication, and catalog mutation. A final post-publication fence recheck rolls back the requesting group link before terminal settlement. Corrupt same-size JPEG-prefix files are decoded before reuse, and repair can replace the immutable-key bytes atomically while preserving links held by other groups. Cache open/unlink tests use a one-shot Unix/macOS race seam for deterministic ancestor and final-component swaps; a `cfg(windows)` final reparse-point containment test compiles for Windows.

The direct acceptance additions cover another selection/group foreign asset, empty and 251-ID service and HTTP requests, mixed-scope ready filtering, missing/corrupt cache repair, multi-group link preservation, source snapshots across hosted wall/screen generation, HTTP missing-file and escaping-symlink rows, and all three HTTP envelopes. Source roots remain read-only; derivative GET remains opaque-ID and managed-cache-only.

Exact round-5 GREEN commands/results:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 14 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_runtime --jobs 2
  PASS: 15 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  PASS: 57 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test task7_source_safety --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 3 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 21 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative --jobs 2
  PASS: 11 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-indexer --test scheduler_priority --jobs 2
  PASS: 13 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check
  PASS: no whitespace errors
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace unit/integration tests and doc-tests; 0 failures
  PASS: app-service lib 99, hosted selections 14, progressive wall 57,
        cache policy 21, cache image derivatives 11, scheduler priority 13,
        derivative HTTP 3, and all remaining workspace suites
```

All Cargo commands were run in the foreground, serially, offline, with `CARGO_BUILD_JOBS=2` and `--jobs 2`; no `cargo clean` or background build was used. The workspace gate was run exactly once, after the final source/test edit and formatting check. After it returned successfully, no source or test file was changed. `pgrep -fl '(^|/)(cargo|rustc)( |$)'` could not enumerate processes on this host (`sysmond service not found`; `Cannot get process list`), so cleanup confirmation is based on every foreground command returning and no background launch.

## Round-5 evidence limits

The publication fence and post-publication rollback establish the exercised in-process admission/repair boundary; they do not claim cross-process transactional atomicity. Unix/macOS tests exercise descriptor-relative ancestor/final-component swaps. The Windows test covers final reparse-point containment compilation and behavior, but Windows ancestor traversal is not claimed to be descriptor-relative. `replace_atomic` has an in-process repair contract; no crash-consistency or cross-process guarantee is claimed.
