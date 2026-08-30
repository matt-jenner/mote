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

## Task 5 exceptional fix round 6

Implementation/test commit: `88c49f8` (`fix: harden hosted derivative publication`), based on `4d2984c`.

This round closes the late-waiter, hosted-supervisor, cache-replacement, bounded-read, and proof gaps found by the breaker review. Hosted commit settlement now drains the current waiter set at the commit boundary and applies a scope authorizer to each waiter. The hosted driver stores and aborts its active child before owner cleanup, uses a hosted-only force-abort transition, and serializes cancellation/publication with one in-process fence; an old attempt cannot publish after terminal failure or overlap a queued successor. Ordinary desktop workers retain their started-blocking-commit cancellation contract.

Unix/macOS cache writes now pin every managed ancestor with descriptor-relative `openat`/`mkdirat`/`renameat`/`linkat`/`unlinkat` operations and `O_NOFOLLOW`; create, replace, and delete ancestor/final-component swaps are deterministic tests with an outside sentinel. Windows has reparse-point-safe parent directory guards, a non-destructive `ReplaceFileW`/`MoveFileExW` replacement seam, and a `cfg(windows)` replacement-failure test. A failed repair leaves old shared bytes and all catalog links intact. The report makes no cross-process catalog transaction claim.

Gallery validation now checks only bounded metadata/JPEG framing with a 64 MiB ceiling and fixed 64 KiB prefix buffer; derivative GET returns a managed asynchronous chunk stream. Sparse oversized and same-prefix malformed files are rejected without full-file buffering or decode. Added proof covers a genuinely foreign library, an actual mixed-scope ready event, shared-link repair/reuse, an HTTP traversal row, cleanup failure injection, and child termination/no stale ready publication after hosted driver cancellation.

Round-6 RED evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib hosted_commit_uses_current_waiters_and_their_commit_scopes --jobs 2
  RED: compile failed because complete_commit_authorized was not yet implemented
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy replacement_failure_preserves_existing_bytes --jobs 2
  RED: compile failed because fail_next_replace_for_test was not yet implemented
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  RED: 55 passed, 2 failed; cancelled_screen_commit_waits_for_started_blocking_work and cancelled_wall_commit_waits_for_started_blocking_work regressed when the initial abort transition was made global
```

The first two commands are behavioral RED probes whose test seams intentionally did not exist before implementation; the third is the genuine behavioral regression used to split hosted force-abort from desktop abort semantics.

Round-6 GREEN evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 25 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative --jobs 2
  PASS: 12 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib --jobs 2
  PASS: 102 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 17 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_runtime --jobs 2
  PASS: 16 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  PASS: 57 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 3 passed, 0 failed
CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check
  PASS: no whitespace errors
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace unit/integration tests and doc-tests; 0 failures
  PASS: app-service lib 102, hosted selections 17, hosted runtime 16,
        progressive wall 57, cache policy 25, cache image derivatives 12,
        scheduler priority 13, derivative HTTP 3, and all remaining suites
```

All Cargo commands were foreground, serial, offline, and used `CARGO_BUILD_JOBS=2`; test commands used `--jobs 2`. No background commands or `cargo clean` were used. The full workspace gate ran exactly once, last, after all source/test edits and formatting. After it returned successfully, no source or test file was changed. This host exposes only the aarch64-apple-darwin Rust target, so the Windows-only compile/run seam was added but not executed locally. Process-table enumeration is restricted; every foreground Cargo command exited and no background build was launched.

## Round-6 evidence limits

Descriptor-relative containment and the publication fence establish the exercised in-process guarantees. Windows parent handles prevent the tested ancestor pathname swap while held and replacement is non-destructive, but Windows and catalog consistency are not claimed as a universal cross-process transaction. The direct hosted commit path intentionally keeps the short cache/catalog publication operation in the owned async attempt under the fence; this avoids a detached blocking publisher, while unusually slow filesystem/catalog operations could still occupy an async worker briefly. GET and cache validation remain bounded and streamed. The Windows-specific tests were not runnable on this macOS-only target.

## Task 5 exceptional fix round 7

Implementation/test commit: `aa66bb7` (`fix: close hosted derivative exceptional races`), based on `06342ab`.

This round closes the remaining exceptional findings. Hosted admission now travels into the blocking encoder and remains held until that work exits, even when the hosted supervisor is aborted; hosted cancellation settles the waiter as `DerivativeUnavailable`, while desktop cancellation behavior is unchanged. Publication rechecks the current many-to-many membership for every waiter under the shared fence, suppressing cache/catalog/event publication when no authorized waiter remains while retaining authorized late waiters and shared-group reuse.

Repair cleanup now removes a final physical file before its final catalog link and restores the recorded bytes when link removal fails; if restoration is impossible, it falls back to deleting the row. Repair paths use `replace_atomic`, and ordinary image generation validates an existing immutable JPEG before allowing generic writer reuse. JPEG validation is bounded to the 64 MiB ceiling and performs full decoding only from controlled blocking work. Derivative GET opens and validates on `spawn_blocking` before handing its descriptor to the existing asynchronous chunk stream.

Windows replacement validation now runs at the replacement boundary with reparse-safe parent handling and no destructive `MoveFileExW` fallback; the cfg-gated seam forces a final reparse swap and checks the outside sentinel and preserved old bytes. Windows execution was not possible because this host has only the `aarch64-apple-darwin` target.

Round-7 RED evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative generate_replaces_an_orphaned_corrupt_regular_file --jobs 2
  RED: failed; generic reuse reported `reused = true` for an orphaned corrupt regular file

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_repair_rejects_same_size_jpeg_with_corrupt_entropy --jobs 2
  RED: failed; marker-only validation accepted the same-size malformed JPEG and the final decode assertion failed

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_membership_removal_at_publication_leaves_no_catalog_link_or_event --jobs 2
  RED: first compile probe failed because `remove_asset_membership` was absent
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_membership_removal_at_publication_leaves_no_catalog_link_or_event --jobs 2
  RED: after adding the mutation fixture, the behavioral probe failed because the catalog row remained after membership removal

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative cached_preview_repair_replaces_an_orphaned_corrupt_regular_file --jobs 2
  RED: failed; cached-preview repair reused the corrupt regular target instead of replacing it
```

Round-7 GREEN evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 21 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative --jobs 2
  PASS: 14 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 25 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib --jobs 2
  PASS: 102 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_runtime --jobs 2
  PASS: 16 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  PASS: 57 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test task7_source_safety --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-indexer --test scheduler_priority --jobs 2
  PASS: 13 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 3 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check
  PASS: no whitespace errors
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace unit/integration tests and doc-tests; 0 failures
  PASS: app-service lib 102, hosted runtime 16, hosted selections 21,
        progressive wall 57, task-7 source safety 1, cache policy 25,
        cache image derivatives 14, scheduler priority 13, derivative HTTP 3,
        and all remaining workspace suites
```

All Cargo commands were run in the foreground, serially, offline, with `CARGO_BUILD_JOBS=2` and `--jobs 2`; no background command or `cargo clean` was used. The full workspace gate ran exactly once, last, after the final source/test edits and formatting. No source or test file changed afterward. Windows API/test code was added under `cfg(windows)` but not executed on macOS; no Windows runtime claim is made. Process-table enumeration is restricted on this host, so cleanup confirmation is based on every foreground command returning and no background launch.

## Round-7 evidence limits

The publication and repair guarantees are in-process fences; they do not claim a cross-process catalog transaction or crash-consistency guarantee. macOS tests exercise the hosted cancellation, membership, cleanup, full JPEG decode, managed streaming, and cache containment behavior. The Windows replacement seam is source-level/cfg-gated only on this host; a Windows build and runtime test remain required on a Windows target.

## Task 5 exceptional fix round 8

Implementation/test commit: `12f88b4` (`fix: close hosted derivative ownership gaps`), based on `f23babc`.

This round closes the remaining hosted cancellation and publication ownership gaps. A per-selection encode guard is moved into the blocking closure, and recovered hosted admission waits for that durable guard to become idle. A cancelled driver therefore cannot admit its queued successor while the old blocking encoder is still running, even when the hosted capacity is greater than one. The publication fence separately records when an attempt owns publication: cancellation before that boundary remains `DerivativeUnavailable`, while a publisher that already captured its authorized waiters retains ownership through terminal settlement instead of dropping those waiters or leaving a committing job behind. Desktop blocking-commit cancellation remains unchanged.

Hosted publication authorization now captures the current authorized waiter set, marks the attempt finishing, stores the allowed scopes, and drains its waiters in one coordinator-locked transition. A racing waiter is either captured by that transition or rejected before it joins a finishing attempt. The empty-to-late regression exercises a membership removal followed by re-addition at deterministic pre-enqueue, encode, publication, and empty-authorization barriers.

Shared derivative cleanup uses an immediate catalog transaction that stages removal of only the requesting group link and reports the remaining link count. Dropping the guard rolls back the link and row changes. Shared bytes remain in place when another link exists. For the final link, the old bytes are bounded and retained until physical deletion and catalog commit finish; a physical-delete failure rolls the transaction back, and a catalog commit failure attempts to restore the retained bytes without deleting unrelated links or rows. The Gallery fault test uses root and child groups, injects requester-link and final physical-delete failures, checks both groups' link counts and old bytes, retries repair/reuse, and verifies the source sentinel.

Cache and Gallery reuse validation now requires the decoded format to be `ImageFormat::Jpeg` before advertising `image/jpeg`. A valid PNG stored at the immutable `.jpg` key is repaired to actual JPEG bytes. On Windows, first-generation repair and replacement now keep the staged file and final parent handles open and publish with `SetFileInformationByHandle(FileRenameInfo)` using the pinned parent handle plus a relative leaf. The destination may be missing, and a raced final reparse entry is replaced as a directory entry rather than followed. The injected Windows failure is placed at the native handle-rename boundary and preserves old and outside sentinel bytes.

Round-8 behavioral RED evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_cancellation_keeps_blocking_encode_admitted_until_successor_runs --jobs 2
  RED: failed with "a successor encoder started before the cancelled encoder exited" when the deterministic hosted admission cap was forced above one

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections authorized_late_waiter_at_empty_commit_boundary_allows_publication --jobs 2
  RED: the late authorized waiter was accepted during early Committing, but the earlier empty scope snapshot settled the request as unavailable instead of publishing

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_repair_reuses_a_shared_derivative_linked_to_another_group --jobs 2
  RED: after injected requester-link removal failure, the shared child-group derivative count was 0 instead of 1 because cleanup deleted the whole derivative row

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative valid_png_at_an_immutable_jpg_key_is_replaced_with_jpeg --jobs 2
  RED: failed with "a PNG must not be reused as image/jpeg"

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_supervisor_drop_restarts_queued_work_and_settles_active_waiter --jobs 2
  RED: failed with "an authorized publication must retain ownership through terminal settlement" because abort dropped the captured delivery after publication authorization
```

The initial cancellation probes used the live scheduler's effective capacity of one and passed, so they are not counted as RED evidence. One later diagnostic was interrupted after its assertion left both artificial encode gates blocked; it is also not counted. The reported cancellation RED is the cleanly exiting, capacity-greater-than-one behavioral run. No Windows RED, compile, or runtime result is claimed.

Round-8 focused GREEN and final evidence:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_cancellation_keeps_blocking_encode_admitted_until_successor_runs --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections authorized_late_waiter_at_empty_commit_boundary_allows_publication --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_repair_reuses_a_shared_derivative_linked_to_another_group --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative valid_png_at_an_immutable_jpg_key_is_replaced_with_jpeg --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_supervisor_drop_restarts_queued_work_and_settles_active_waiter --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections hosted_driver_drop_aborts_active_attempt_without_stale_publication --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-catalog --test catalog_round_trip requester_link_removal_rolls_back_or_preserves_shared_derivative_links --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-catalog --test catalog_round_trip --jobs 2
  PASS: 13 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 25 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative --jobs 2
  PASS: 15 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections corrupt_hosted_thumbnail_is_repaired_before_it_is_reused --jobs 2
  PASS: 1 passed, 0 failed, including same-size valid PNG repair
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 22 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall cancelled_ --jobs 2
  PASS: 3 passed, 0 failed, including both desktop started-blocking-work cancellation tests
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --lib derivative_coordinator --jobs 2
  PASS: 49 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_runtime --jobs 2
  PASS: 16 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 3 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo fmt --all
  PASS: formatting completed
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check
  PASS: no whitespace errors
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace unit/integration tests and doc-tests; 0 failures
  PASS: app-service lib 102, hosted runtime 16, hosted selections 22,
        progressive wall 57, task-7 source safety 1, cache policy 25,
        cache image derivatives 15, catalog round trip 13,
        derivative HTTP 3, and all remaining workspace suites
```

All Cargo commands were run serially in the foreground, offline, with `CARGO_BUILD_JOBS=2`; test commands used `--jobs 2`. No background build or `cargo clean` was used. The full workspace gate ran exactly once and was the final Cargo command, after all source/test edits and formatting. No source or test file changed afterward. Every foreground command exited. Process enumeration remains restricted on this host, and no background Cargo/rustc process was launched.

## Round-8 evidence limits

`rustup target list --installed` reported only `aarch64-apple-darwin`. The `cfg(windows)` implementation and tests were therefore not compiled or run here. Static review checked the `SetFileInformationByHandle` signature, `FileRenameInfo` field offsets/alignment, `RawHandle` use, required staged-file delete access/share modes, pinned parent handle, relative UTF-16 leaf length, missing-destination behavior, and non-destructive failure paths. A Windows target build and runtime execution are still required; no Windows runtime evidence is claimed. The catalog guard is an in-process API backed by one SQLite immediate transaction plus compensating byte restoration. This report does not claim a cross-process cache/catalog transaction or crash consistency. Source roots remain read-only, and all physical mutation stays beneath the canonical managed cache.

## Task 5 exceptional fix round 9

Implementation/test commit: `4aa7876` (`fix: linearize hosted attempt handoff`), based on `5555643`.

This round replaces the stale publication flag snapshot and the non-atomic runtime encode-idle check with one coordinator-created lease stored on the hosted work ticket. The lease moves through `PreEncode`, `Encoding`, `PrePublication`, `Publishing`, and `Finished` using short synchronous transitions. Its encode guard is owned by the blocking closure, while its task guard records completion of the async child. Cancellation either wins before publication and prevents any later encode registration, or observes `Publishing` and leaves the captured publication delivery alive. Driver generation is released only after the lease is finished, so an already-started blocking encoder exits before successor handoff. No mutex is held across blocking encode work or an async wait. Desktop commit cancellation remains on its existing path.

Two debug-only deterministic barriers exercise the previously uncovered transitions. The publication regression pauses driver drop after the old snapshot point, allows the attempt to acquire the fence and authorize publication, then resumes cancellation; it checks successful typed settlement, the ready event, catalog state, zero retained coordinator jobs, and a source sentinel. The encode regression pauses before registration, completes cancellation and successor ownership handoff, then requires the abort-marked attempt to report `rejected` rather than `registered`; it also checks exact `DerivativeUnavailable` settlement and successor completion at hosted capacity four.

Round-9 behavioral RED evidence:

```text
CARGO_BUILD_JOBS=2 cargo test -p photo-app-service --test hosted_selections hosted_cancellation_ --offline --jobs 2 -- --nocapture
  RED: 1 passed, 2 failed
  FAIL: hosted_cancellation_transition_yields_to_publication_authorized_after_drop_starts
        stale false publication snapshot aborted the already-authorized delivery
  FAIL: hosted_cancellation_rejects_encode_registration_after_successor_handoff
        abort-marked attempt registered blocking work after successor handoff
  PASS: the existing already-entered blocking-encode cancellation regression
```

Round-9 focused GREEN and final evidence:

```text
CARGO_BUILD_JOBS=2 cargo test -p photo-app-service --test hosted_selections hosted_cancellation_ --offline --jobs 2 -- --nocapture
  PASS: 3 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test -p photo-app-service --test hosted_selections --offline --jobs 2
  PASS: 24 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test -p photo-app-service --lib --offline --jobs 2
  PASS: 102 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test -p photo-app-service --test progressive_wall cancelled_ --offline --jobs 2
  PASS: 3 passed, 0 failed, including both desktop started-blocking-work cancellation tests
CARGO_BUILD_JOBS=2 cargo test -p photo-app-service --test hosted_runtime --offline --jobs 2
  PASS: 16 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test -p photo-server --test derivative_api --offline --jobs 2
  PASS: 3 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo fmt --all
  PASS: formatting completed
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check
  PASS: no whitespace errors
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace unit/integration tests and doc-tests; 0 failures
  PASS: app-service lib 102, hosted selections 24, hosted runtime 16,
        progressive wall 57, task-7 source safety 1, cache policy 25,
        cache image derivatives 15, catalog round trip 13,
        derivative HTTP 3, and all remaining workspace suites
```

All Cargo commands ran serially in the foreground and offline with `CARGO_BUILD_JOBS=2`; test commands used `--jobs 2`. No background command or `cargo clean` was used. The full workspace gate ran exactly once and was the final Cargo command, after all source/test edits, formatting, and the implementation/test commit. No source or test file changed afterward, and every foreground Cargo command exited. Process enumeration is restricted on this host; cleanup confirmation is based on those exited foreground sessions and the fact that no background build was launched.

## Round-9 evidence limits

The two new regressions are macOS-executed in-process hosted cancellation tests. Round 9 did not change Windows-specific code and does not claim Windows compile or runtime evidence. It also does not claim a cross-process cache/catalog transaction. The lease closes the in-process supervisor/attempt ownership transitions under review while preserving the existing managed-cache, source-read-only, and desktop contracts.
