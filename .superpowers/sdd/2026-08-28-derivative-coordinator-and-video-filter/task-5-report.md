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

## Fix round 1

### Review findings addressed

- Collection advances and phase finishes now use an exact `CollectionProgressToken` containing selection, phase, cursor, and background generation. A stale token is rejected atomically; visible invalidation rewinds even an in-progress thumbnail cursor, and thumbnail/preview completion resets the cursor in the same state transition as the phase change.
- Recent-only background drivers return without screen work while the collection is dormant or in the thumbnail phase. The full-group driver can therefore acquire the collection mutex after a recent driver without a starvation loop.
- Terminal-warning clearing now checks both current wall and screen terminal rows for the current asset key and availability before removing the shared warning. Both clear directions are covered.
- Restart tests open the same catalogue/cache without startup reconciliation, verify zero attempts for an unchanged terminal key/availability, then verify exactly one changed-key attempt and exactly one availability-recovery attempt. Rows and warnings converge after successful recovery.
- Screen encode instrumentation increments at the encode boundary, before the blocking encoder call. The 301-photo phase test asserts zero starts while the late wall is held.
- The 10k bound test now runs the collection scheduler with missing wall work held at a deterministic gate while a foreground request is queued, and samples page/recent/queue bounds with the foreground slot included.

### RED evidence

The first token test failed to compile because the exact token API did not exist. After the API was present, the warning regression failed as expected (`left: 0`, `right: 1`) when clearing one class removed the shared warning. The 301 test likewise first failed to compile because the encode-boundary counter API did not exist. These failures preceded their corresponding production changes.

### GREEN evidence

Focused gates:

```text
$ cargo test -p photo-app-service --test progressive_wall every_photo_page_reaches_wall_outcome_before_background_screens_begin -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_photo_failures_and_videos_do_not_starve_healthy_previews -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_failure_retries_only_after_derivative_key_changes -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_failure_retries_when_availability_changes_without_a_new_key -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_failure_restart_skips_same_key_and_retries_changed_key_once -- --exact
test result: ok. 1 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall terminal_failure_restart_skips_same_availability_and_retries_after_recovery_once -- --exact
test result: ok. 1 passed; 0 failed
```

Relevant suites and quality gates:

```text
$ cargo test -p photo-app-service --lib -- --test-threads=1
test result: ok. 58 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
test result: ok. 54 passed; 0 failed

$ cargo test -p photo-catalog --test wall_query -- --test-threads=1
test result: ok. 19 passed; 0 failed

$ cargo test -p photo-catalog --test catalog_round_trip -- --test-threads=1
test result: ok. 12 passed; 0 failed

$ cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
exit=0

$ git diff --check
exit=0
```

Workspace and release gates:

```text
$ cargo test --workspace --all-targets -- --test-threads=1
all workspace test targets passed

$ cargo check --workspace --release
Finished successfully

$ cargo test -p photo-app-service --lib --release -- --test-threads=1
test result: ok. 53 passed; 0 failed
```

### Boundedness and source safety

`collection_schedules_bounded_missing_work_with_concurrent_foreground_request` exercises a real first collection page of missing wall work and queues a concurrent visible request. Its snapshot asserts `largest_loaded_page <= 250`, `recent_len <= 250`, and `queued_jobs <= 251`; the existing 10k pagination test remains green. Collection code retains only the current page and opaque cursor, and the recent window remains capped at 250. No source or cache paths are added to DTOs, test counters, tokens, warnings, or coordinator snapshots.

### Commit

Fix-round implementation commit: `57aed3b87d81316572f344c8b0624c39b8a4fb6b` (`fix: guard collection phase transitions and terminal warnings`).

## Fix round 2

### Review findings addressed

- Collection/recent background enqueue now carries the complete `CollectionProgressToken`. The coordinator validates selection, phase, cursor, and background generation in the same lock that inserts a job, so a stale caller cannot enqueue work stamped with the current generation or clear stale terminal state before admission.
- Cached collection/recent publication now uses a coordinator admission boundary. The small synchronous publication runs while the token is admitted; invalidation before admission suppresses it, while invalidation after admission is ordered after it. Idle preview generation uses the same exact-token enqueue boundary and rechecks the token before catalog completion/phase advancement.
- Consumed recent IDs are removed only through the same current collection token, preventing a stale recent driver from mutating the rewound window.
- Visible wall requests restart the full-group driver after a rewind so cached screen references are replayed through the wall-first phases; ordinary near-viewport requests remain recent-only.
- The 10k bound test now uses a distinct `NearViewport` wall request, which does not invalidate collection work. The first real 250-item missing-wall page remains held at its deterministic gate while the foreground lane is admitted, and the snapshot is taken before release/cancellation.

### RED evidence

The new four interleaving tests were added before the collection admission APIs existed. The focused test compile failed with missing `install_collection_*_test_gate` and coordinator guarded-enqueue methods, which was the expected pre-fix failure. The corrected tests then exercised cached publication and enqueue gaps independently for recent and full-page drivers.

### GREEN evidence

Focused interleavings and bounds:

```text
$ cargo test -p photo-app-service --lib stale_ -- --test-threads=1
test result: ok; 8 passed; 0 failed

$ cargo test -p photo-app-service --lib collection_schedules_bounded_missing_work_with_concurrent_foreground_request -- --test-threads=1
test result: ok; 1 passed; 0 failed

$ stale interleavings repeated 20 times
all 20 runs passed
```

Relevant suites and quality gates:

```text
$ cargo test -p photo-app-service --lib -- --test-threads=1
test result: ok; 62 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
test result: ok; 54 passed; 0 failed

$ cargo test -p photo-catalog --test wall_query -- --test-threads=1
test result: ok; 19 passed; 0 failed

$ cargo test -p photo-catalog --test catalog_round_trip -- --test-threads=1
test result: ok; 12 passed; 0 failed

$ cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
exit=0

$ git diff --check
exit=0
```

Workspace and release gates:

```text
$ cargo test --workspace --all-targets -- --test-threads=1
all workspace test targets passed

$ cargo check --workspace --release
Finished successfully

$ cargo test -p photo-app-service --lib --release -- --test-threads=1
test result: ok; 53 passed; 0 failed
```

### Boundedness and source safety

The corrected 10k test schedules the collection's actual first 250-item wall page and a separate near-viewport foreground request, then samples before either deterministic gate is released. It asserts `largest_loaded_page <= 250`, `recent_len <= 250`, and `queued_jobs <= 251` (250 collection slots plus the one foreground request). The collection driver retains only its current page and cursor; recent IDs remain capped at 250. Collection tokens, test snapshots, warnings, and updates contain no native source/cache paths.

### Commit

Fix-round 2 implementation commit: `db56f6e82f8fe625ffa6170fcc9bc8f3c92ca9af` (`fix: guard stale collection admissions`).

## Fix round 3

### Review finding addressed

The 10k bound test now installs a class-wide wall gate before starting the
real collection driver. Because the first page is loaded with the catalogue's
250-item keyset limit and all page jobs are admitted before workers start, the
gate holds every wall attempt from that page. The test waits for a wall attempt
and asserts an exact 250-job coordinator snapshot, rather than observing only
one blocked asset while other workers drain the page. It then replaces the
gate with a separate per-asset wall gate, submits a non-invalidating
near-viewport request, waits for its queue admission, and asserts exactly 251
jobs before releasing either gate.

The fixture itself no longer retains a 10,000-element ID vector; records are
flushed in bounded 500-record batches and the foreground ID is derived from
its known relative path. The test also reads the catalogue count and asserts
exactly 10,000 assets. The production snapshot remains path/ID-free and
reports only recent/page/job counts and phase.

### GREEN evidence

```text
$ cargo test -p photo-app-service --lib collection_schedules_bounded_missing_work_with_concurrent_foreground_request -- --test-threads=1
test result: ok. 1 passed; 0 failed

$ focused bound test repeated 20 times
20/20 bound runs passed

$ cargo test -p photo-app-service --lib -- --test-threads=1
test result: ok. 62 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
test result: ok. 54 passed; 0 failed

$ cargo test -p photo-catalog --test wall_query -- --test-threads=1
test result: ok. 19 passed; 0 failed

$ cargo test -p photo-catalog --test catalog_round_trip -- --test-threads=1
test result: ok. 12 passed; 0 failed

$ cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
exit=0

$ git diff --check
exit=0

$ cargo test --workspace --all-targets -- --test-threads=1
workspace_exit=0
```

### Boundedness and source safety

The strengthened test proves `largest_loaded_page == 250`,
`recent_len <= 250`, and `queued_jobs == 250` while every collection wall
attempt is gated. After the independent near-viewport request is admitted it
proves `queued_jobs == 251`, accounting for exactly one foreground job. Gates
are released and the spawned tasks are joined/aborted before the coordinator
cleanup, so no deterministic work is left running. The fixture checks the
catalogue's 10,000-asset count while retaining only bounded insertion records;
collection runtime state still consists of one page and one cursor, with no
full ID collection exposed or retained by the coordinator snapshot.

No source or cache paths were added to the test hook, snapshot, queue state,
or report-facing data.

### Commit

Fix-round 3 test-hardening commit: `a8d0eefd5b40a4abd23bf93638ac9baa4e5370ee` (`test: make collection bound gate deterministic`).

## Fix round 4

### Review findings addressed

- The 10k bound test now waits for the literal configured derivative worker
  count (`MAX_DERIVATIVE_WORKERS`, two) in the class-wide wall gate's atomic
  entry counter before replacing that gate. Both active workers have therefore
  cloned the original gate and remain held by its retained release handle;
  the separate per-asset foreground gate cannot race either worker's gate
  snapshot.
- Debug/test-only derivative task tracking now covers driver tasks, attempt
  tasks, attempt monitors, and admitted-commit operation/monitor tasks. A
  count-and-notify guard decrements on cancellation, panic, or normal return.
  `wait_for_zero` creates its notification future before the atomic count
  check and uses `notify_one`, which preserves a permit across the
  check/register window and avoids a lost wakeup. Cleanup releases both gates,
  aborts caller futures, aborts coordinator attempts, invalidates queued
  background work, waits for zero active derivative tasks, and asserts both
  zero active tasks and zero coordinator jobs. No fixed delay remains.

### GREEN evidence

```text
$ cargo test -p photo-app-service --lib collection_schedules_bounded_missing_work_with_concurrent_foreground_request -- --test-threads=1
test result: ok. 1 passed; 0 failed

$ focused bound test repeated 100 times (8 concurrent processes)
100-run-exit=0

$ cargo test -p photo-app-service --lib -- --test-threads=1
test result: ok. 62 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
test result: ok. 54 passed; 0 failed

$ cargo test -p photo-catalog --test wall_query -- --test-threads=1
test result: ok. 19 passed; 0 failed

$ cargo test -p photo-catalog --test catalog_round_trip -- --test-threads=1
test result: ok. 12 passed; 0 failed

$ cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
exit=0

$ git diff --check
exit=0

$ cargo check --workspace --release
Finished successfully

$ cargo test --workspace --all-targets -- --test-threads=1
workspace_exit=0
```

### Boundedness and cleanup evidence

The test's first snapshot still proves `largest_loaded_page == 250`,
`recent_len <= 250`, and `queued_jobs == 250` with both configured workers
held. After the independent near-viewport admission it proves exactly 251
jobs. After releasing, cancelling, and invalidating, it awaits tracked
quiescence and proves `derivative_active_task_count_test() == 0` and
`queued_jobs == 0`. The tracker exposes only an integer count and a wait
operation; it stores no asset IDs, source paths, or cache paths. The bounded
10k fixture continues to flush insertion records in 500-record batches and
retains no full ID vector.

### Commit

Fix-round 4 test-hardening commit: `e7f134c183c319a9f3be9f54689ae2fbfc6671c7` (`test: await derivative worker quiescence`).

## Fix round 5

### Review findings addressed

- Every deterministic derivative gate now creates, pins, and enables its
  release notification future before incrementing or signaling entry. This
  closes the entry-signal/release-registration gap for class-wide and
  foreground derivative gates, collection publish/enqueue gates, post-encode
  and post-admission gates, visible-request hooks, and commit-delivery hooks.
- Debug/test-only derivative quiescence now registers each waiter before its
  active-count check and broadcasts the transition to zero with
  `notify_waiters`. A deterministic regression starts two simultaneous
  `wait_for_zero` callers behind one active guard and proves both complete
  after the guard drops; the waiter count returns to zero.

### GREEN evidence

```text
$ cargo test -p photo-app-service --lib derivative_task_tracker_broadcasts_quiescence_to_all_waiters -- --test-threads=1
test result: ok; 1 passed; 0 failed

$ cargo test -p photo-app-service --lib collection_schedules_bounded_missing_work_with_concurrent_foreground_request -- --test-threads=1
test result: ok; 1 passed; 0 failed

$ focused tracker and bound tests repeated 100 times each (8 concurrent processes)
tracker_rc=0 bound_rc=0

$ cargo test -p photo-app-service --lib -- --test-threads=1
test result: ok; 63 passed; 0 failed

$ cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
test result: ok; 54 passed; 0 failed

$ cargo test -p photo-catalog --test wall_query -- --test-threads=1
test result: ok; 19 passed; 0 failed

$ cargo test -p photo-catalog --test catalog_round_trip -- --test-threads=1
test result: ok; 12 passed; 0 failed

$ cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
Finished successfully

$ cargo fmt --all -- --check
exit=0

$ git diff --check
exit=0

$ cargo check --workspace --release
Finished successfully

$ cargo test -p photo-app-service --lib --release -- --test-threads=1
test result: ok; 53 passed; 0 failed

$ cargo test --workspace --all-targets -- --test-threads=1
workspace_exit=0
```

### Liveness, boundedness, and source safety

The gate changes are test/debug harness synchronization only; they do not
change production scheduling. Notification futures are armed before any
entry signal, so a release cannot be lost in the deterministic harness.
Quiescence completion is broadcast to all registered waiters, and the
two-waiter regression proves no caller remains stranded. The bound test still
holds all configured wall workers, admits the independent foreground request,
and proves `largest_loaded_page == 250`, `recent_len <= 250`, and exactly
`queued_jobs == 251` before release. Cleanup waits for tracked derivative
quiescence and zero coordinator jobs without an arbitrary sleep. Counters and
test snapshots expose only bounded counts; no asset IDs, source paths, or
cache paths are retained or reported.

### Commit

Fix-round 5 implementation commit: `6bcc9797c33e688e3592a91fb7bca5837e020b58` (`test: close derivative gate wakeup races`).
