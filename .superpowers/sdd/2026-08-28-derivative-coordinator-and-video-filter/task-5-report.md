# Task 5 report: bounded thumbnail-first collection work

## Implementation summary

- Added a coordinator-owned thumbnail phase followed atomically by a preview phase. Each phase advances one photo-only keyset page at a time and never starts the next page until every wall item is ready or has a persisted terminal outcome.
- Added terminal handling for source I/O/decode failures. The terminal identity includes the current derivative key and availability, so unchanged failures are skipped across restarts while a changed key or recovered availability is eligible again. Successful derivatives clear the terminal row and public warning.
- Kept cache-wide failures retryable with bounded exponential backoff; they do not create asset terminal rows.
- Preserved indexed videos in catalogue/scanner state while filtering them from collection pages, derivative resolution, recent work, derivative queues, and wall publications.
- Paused idle wall/preview lanes while interaction is active, while retaining foreground and near-viewport lanes. Collection invalidation rewinds to the thumbnail phase safely, and consumed recent IDs are removed conditionally so concurrent foreground work is not lost.
- Added debug-only boundedness counters and tests for a 10,000-asset catalogue, plus terminal invalidation, availability recovery, mixed-media, and non-starvation coverage.

## RED evidence

These exact tests were run before the implementation changes and failed for the expected reasons:

```text
$ cargo test -p photo-app-service --test progressive_wall terminal_photo_failures_and_videos_do_not_starve_healthy_previews -- --exact
test terminal_photo_failures_and_videos_do_not_starve_healthy_previews ... FAILED
timed out waiting for healthy screen previews

$ cargo test -p photo-app-service --test progressive_wall terminal_wall_failure_persists_a_nonretryable_warning -- --exact
test terminal_wall_failure_persists_a_nonretryable_warning ... FAILED
assertion failed: terminal warning must be non-retryable
```

The existing 301-item phase-order test was also renamed to the exact Task 5 acceptance name before the green gate.

## GREEN evidence

Exact phase, starvation, key-change, and availability-change gates:

```text
$ cargo test -p photo-app-service --test progressive_wall every_photo_page_reaches_wall_outcome_before_background_screens_begin -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_photo_failures_and_videos_do_not_starve_healthy_previews -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_failure_retries_only_after_derivative_key_changes -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_failure_retries_when_availability_changes_without_a_new_key -- --exact
test result: ok. 1 passed; 0 failed
```

Focused suites and quality gates:

```text
$ cargo test -p photo-app-service --lib -- --test-threads=1
test result: ok. 53 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
test result: ok. 52 passed; 0 failed

$ cargo test -p photo-catalog --test wall_query -- --test-threads=1
test result: ok. 19 passed; 0 failed

$ cargo test -p photo-catalog --test catalog_round_trip -- --test-threads=1
test result: ok. 12 passed; 0 failed

$ cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
Finished `dev` profile

$ cargo fmt --all -- --check
exit=0

$ git diff --check
exit=0
```

Workspace and release checks:

```text
$ cargo test --workspace --all-targets -- --test-threads=1
all workspace test targets passed

$ cargo check --workspace --release
Finished `release` profile

$ cargo test -p photo-app-service --lib --release -- --test-threads=1
test result: ok. 52 passed; 0 failed
```

## Boundedness evidence

- `remaining_group_ids_page` always calls `photo_asset_ids_page(..., 250)` and does not retain page IDs after returning/advancing the cursor.
- The recent window, coordinator terminal-key window, and completed-outcome history are each capped at 250 entries.
- `remaining_idle_group_enumeration_stays_bounded_for_ten_thousand_assets` inserts and paginates 10,000 assets, asserts every page is at most 250, consumes all 10,000 by cursor, and asserts the debug snapshot has `recent_len <= 250`, `largest_loaded_page <= 250`, and `queued_jobs <= 250`.
- The coordinator recent-window test feeds 10,000 IDs and still observes exactly the bounded 250-entry window. Snapshot data contains only counts and phase; it exposes no asset IDs or paths.

## Source-safety evidence

Warnings and updates use opaque selection/library/asset identifiers and path-free messages. Derivative references expose only asset IDs, derivative classes, and opaque cache keys. Native source and cache paths remain confined to internal catalogue/cache operations. Videos remain indexable and continue through scanner progress accounting, but do not enter photo wall pages or derivative work.

## Commit

Implementation commit: `40baab9bb3d3800bc64d2ee57c52bd9133d8150b` (`feat: run thumbnail-first collection phases`).
