# Task 3 report

## Files

Added the selection-explicit `GalleryEngine`, shared `SelectionRuntime`, bounded replaying and filtering subscriptions, stable selection summaries, scope-bound wall cursors, and focused hosted selection/runtime tests. Desktop cursor callers remain compatible with the existing `AppService` wrapper.

## RED/GREEN

RED was observed with `cargo test -p photo-app-service --test hosted_selections --no-run`: compilation failed because `GalleryEngine` did not exist. GREEN was observed after implementation with the focused hosted selection/runtime tests passing.

## Verification

`cargo test -p photo-app-service --test hosted_selections --test hosted_runtime` passed.

`cargo test -p photo-app-service` passed, including the desktop regression and source-safety suites.

`cargo fmt --all` and `git diff --check` passed.

Base implementation commit: `4f46c88` (`feat: run hosted galleries by explicit selection`).

## Concerns

Derivative request routing remains owned by the later derivative task; this checkpoint provides the shared per-selection coordinator and event infrastructure for that work. Runtime history uses a synchronous mutex so the synchronous subscription constructor can snapshot replay state without blocking an async executor.

## Fix round 1 evidence

Regression-first tests were added for isolated scheduler namespaces, canonical selection-ID aliases, and reconnect demand ownership. The namespace test was observed RED before the owner-prefix change and GREEN afterward; the alias and reconnect tests were likewise observed RED before their corresponding fixes.

The fix round preserves the base checkpoint at `4f46c88` and is committed separately with the final verification results below. The report intentionally identifies the immutable base SHA; the fix-round commit is the commit containing this report.

The scan path now keeps the actual cancellation sender, flushes progressive batches after 50 ms, propagates catalog batch failures, avoids completing failed generations, and emits `catalogUnavailable` without native paths. Concurrent runtime publishes serialize history insertion and broadcast delivery, synthetic resyncs do not advance the event head, dead runtime registry entries are pruned, and subscription demand removal is token-owned.

Fix-round implementation commit: `228743a` (`fix: harden hosted selection runtimes`).

Fix-round verification passed: `cargo test -p photo-app-service`, `cargo test --workspace`, `cargo fmt --all -- --check`, and `git diff --check`.

## Fix round 2 evidence

RED/GREEN evidence: `owned_dequeue_finds_its_best_job_behind_a_foreign_head` failed before the scheduler ownership scan and passed afterward. The alias, reconnect-demand, and escaping-symlink regressions similarly failed against the pre-fix behavior and pass with the fixes.

Round 2 changes make `IndexScheduler::next_owned` select the highest-priority job belonging to its owner even behind a foreign heap head, serialize subscription publication snapshots against event publication, retain one demand entry per live subscription token, add settled-scan idempotence, share enrichment admission across scanners (including shape work) at the scheduler's configured global capacity, and canonicalize selected filesystem paths before catalog/runtime creation. Final-subscription drop now sends the actual scan cancellation sender when demand is empty, and a completed scan removes its dead registry entry when no subscriber retains the runtime.

Round 2 implementation commit: `06a4e3f` (`fix: enforce hosted runtime ownership and admission`).

Round 2 lifecycle follow-up commit: `3cdb520` (`fix: close hosted runtime lifecycle gaps`).

Round 2 focused verification passed: `cargo test -p photo-indexer --test scheduler_priority`, `cargo test -p photo-app-service --test hosted_selections --test hosted_runtime`, and `cargo check -p photo-indexer -p photo-app-service`.

Additional regression verification passed: the blocking foreground/indexing desktop test, both scanner admission interaction tests, and the new hosted-runtime unit tests for cancellation ownership and concurrent publication/history ordering.

Remaining architectural concern: the existing desktop `AppService` still owns its historical active-selection coordinator/scanner path; fully replacing it with `GalleryEngine` requires moving its extensive derivative test hooks and reconciliation lifecycle together. The public desktop regression suite remains green, but this extraction is not represented as a completed change in this round.

Final HEAD semantics: after the verified implementation commits above, the final report commit is the branch HEAD and contains this report; `b30a481` is the latest implementation commit.

## Fix round 3 evidence

RED/GREEN evidence: the offline reopen regression first failed because `GalleryEngine::open` rejected the missing `/var` alias before resolving the cataloged root; it passes after canonicalizing the nearest existing ancestor. The in-root symlink identity regression passes after selection persistence uses the canonical root-relative target. The expired-lease unit regression passes after connected demand was decoupled from the interaction lease. Runtime publication and scheduler regressions remain green.

Round 3 changes derive newly-created runtime settlement from authoritative catalog generation state, preserve cached wall access while the source is offline, canonicalize missing-root identities and in-root selections, retain connected scope demand after lease expiry, and remove the runtime registry entry on the final subscription drop without cancelling the primary scan. The primary scan cancellation sender remains available for persistence failure handling; final subscriber drop does not cancel admitted generation work.

Round 3 implementation commit: `b30a481` (`fix: preserve hosted runtime ownership across restart`). The desktop extraction remains blocked by the existing `AppService`'s coupled historical scanner/coordinator/derivative/reconciliation state and requires a coherent cross-module migration to avoid duplicating catalog connections or breaking the public desktop lifecycle. This blocker is recorded explicitly for fresh-agent escalation; no contradictory desktop-completion claim is made.

## Fix round 4 evidence

RED/GREEN evidence: the settled offline regression was observed RED when a completed catalog skipped source inspection and is GREEN after `ensure_running` performs a cheap availability check and publishes one path-free `SourceUnavailable` while cached wall rows remain queryable. The live source-root symlink retarget regression was observed RED when display-path fallback rebound the old library and is GREEN after canonical roots are preferred whenever the configured source currently resolves. The progressive desktop refresh regression was observed RED when a same-folder refresh reused settled runtime state and is GREEN after desktop epochs create an explicit fresh runtime. The desktop wrapper, replay, lag, scope-filtering, drop-order, stream-survival, admission, and scan-count regressions all pass with deterministic notifications/barriers; the former 1 ms cleanup poll is replaced by an observable runtime lifecycle signal (now a `watch` state channel).

Round 4 changes extract desktop scanning/reconciliation into the shared `GalleryEngine` and `SelectionRuntime` without a second catalog or coordinator state. `AppService` now injects its existing state, scheduler, metadata reader, and derivative coordinator into the engine; desktop scan, wall query, scope interaction, cancellation, and runtime-count paths use that wrapper. Runtime completion and offline termination clear the legacy active marker through an observable signal. Replay snapshots remain publication-serialized, and lag resync watermarks are monotonic. New deterministic coverage includes same/different-selection scan starts, mixed-scope catalog/derivative/warning/clear filtering, retained replay edges plus backlog/live boundaries, broadcast lag, registry ownership in both drop orders, primary-scan survival after stream drop, cross-selection admission, settled offline reporting, current-folder in-root symlink walls, and the desktop wrapper.

Round 4 implementation commit: `5ded34c` (`fix: complete hosted gallery runtime extraction`).

Round 4 focused verification passed:

- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo check -p photo-app-service`.
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-indexer --test scheduler_priority` (11/11).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service --test hosted_selections --test hosted_runtime --test desktop_gallery_wrapper` (6 + 9 + 1 tests).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service --test progressive_wall` (56/56).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service` (79 unit tests and all app-service integration/doc tests).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test --workspace` (all workspace tests and doc tests).
- `cargo fmt --all -- --check` and `git diff --check`.

The round-4 implementation and report are intentionally separate commits so this report records the immutable implementation SHA; the report-only commit is the branch HEAD after this append.

## Fix round 5 evidence

Round 5 closes the remaining desktop admission and hosted-runtime lifecycle races. The scan control is now a synchronous, short critical-section state machine with a `watch` lifecycle signal. Admission, cancellation, cancellation-sender installation, and terminal completion are coordinated atomically; cancellation no longer uses `try_lock`, and every canceled scan publishes `Cancelled` so bridges and cleanup watchers can terminate and release their runtime. Desktop bridge installation and forwarding are serialized with persisted active-selection, protected-group, and `selection_epoch` changes. Existing hosted subscriptions receive an authoritative scope watch, so filtering and aggregate demand change together when client interaction changes from `CurrentFolder` to `IncludeSubfolders` or back.

Regression coverage added in this round includes the deterministic stale async token barrier (`stale_async_scan_token_is_rejected_before_bridge_or_scan_admission`), cancellation under scan-control lock contention, multiple lifecycle waiters for both cancellation and completion, exact-once concurrent scan admission, canceled desktop bridge termination/runtime release, both runtime-registry drop orderings, mutable-scope filtering for catalog/derivative/warning/clear events, and a bounded progressive flush assertion. The progressive test has a 250 ms timeout and no wall-clock sleep; the implementation's progressive batch deadline remains 50 ms.

Fix round 5 implementation commit: `c5a7055` (`fix: close hosted runtime lifecycle races`).

Fix round 5 focused verification passed:

- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service service::tests:: -- --nocapture` (6/6).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service hosted_runtime::tests:: -- --nocapture` (6/6).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-indexer --test scheduler_priority` (11/11).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service --test hosted_selections --test hosted_runtime --test desktop_gallery_wrapper` (6 + 12 + 1).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service --test progressive_wall` (57/57).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test -p photo-app-service` (85 unit tests and all app-service integration/doc tests).
- `CARGO_TARGET_DIR=/Users/jennerm/repos/photo_viewer/target cargo test --workspace` (all workspace tests and doc tests).
- `cargo fmt --all -- --check` and `git diff --check`.

The report remains separate from the implementation commit; the report-only commit is the branch HEAD after this append.
