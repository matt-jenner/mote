# Task 3 report

## Files

Added the selection-explicit `GalleryEngine`, shared `SelectionRuntime`, bounded replaying and filtering subscriptions, stable selection summaries, scope-bound wall cursors, and focused hosted selection/runtime tests. Desktop cursor callers remain compatible with the existing `AppService` wrapper.

## RED/GREEN

RED was observed with `cargo test -p photo-app-service --test hosted_selections --no-run`: compilation failed because `GalleryEngine` did not exist. GREEN was observed after implementation with the focused hosted selection/runtime tests passing.

## Verification

`cargo test -p photo-app-service --test hosted_selections --test hosted_runtime` passed.

`cargo test -p photo-app-service` passed, including the desktop regression and source-safety suites.

`cargo fmt --all` and `git diff --check` passed.

Commit: `bd35010` (`feat: run hosted galleries by explicit selection`).

## Concerns

Derivative request routing remains owned by the later derivative task; this checkpoint provides the shared per-selection coordinator and event infrastructure for that work. Runtime history uses a synchronous mutex so the synchronous subscription constructor can snapshot replay state without blocking an async executor.

## Fix round 1 evidence

Regression-first tests were added for isolated scheduler namespaces, canonical selection-ID aliases, and reconnect demand ownership. The namespace test was observed RED before the owner-prefix change and GREEN afterward; the alias and reconnect tests were likewise observed RED before their corresponding fixes.

The fix round preserves the base checkpoint at `4f46c88` and is committed separately with the final verification results below. The report intentionally identifies the immutable base SHA; the fix-round commit is the commit containing this report.

The scan path now keeps the actual cancellation sender, flushes progressive batches after 50 ms, propagates catalog batch failures, avoids completing failed generations, and emits `catalogUnavailable` without native paths. Concurrent runtime publishes serialize history insertion and broadcast delivery, synthetic resyncs do not advance the event head, dead runtime registry entries are pruned, and subscription demand removal is token-owned.

Fix-round implementation commit: `228743a` (`fix: harden hosted selection runtimes`).

Fix-round verification passed: `cargo test -p photo-app-service`, `cargo test --workspace`, `cargo fmt --all -- --check`, and `git diff --check`.
