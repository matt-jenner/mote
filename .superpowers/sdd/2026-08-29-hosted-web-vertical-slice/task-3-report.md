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
