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

Final HEAD semantics: after the verified implementation commits above, the final report commit is the branch HEAD and contains this report; `3cdb520` is the latest implementation commit.
