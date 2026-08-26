# Task 3 report: deterministic interaction-mode admission test

## Status

Complete. The repair is test-only and local to `crates/indexer/tests/progressive_scan.rs`.

## Root cause

`enrichment_admission_tracks_mode_changes_during_one_scan` used an `AdmissionReader` whose gate was permanently opened after the first two reads. Once those reads completed, later reads could both finish and surrender scheduler admission before the assertion observed the third start. A legal fourth start then made the exact start-count assertion race normal execution.

## RED evidence

The existing test was run in ten isolated attempts before the repair; attempt 5 reproduced the failure:

```text
assertion `left == right` failed
left: 4
right: 3
```

The extra start was a normal post-release read completing before the assertion, not a production admission violation.

## Implementation

- Added a small local `PermitMetadataReader` with a per-read permit count and start notifications.
- The test starts the scan with two idle workers and waits until both per-read permits are blocked before switching the shared scheduler to active mode.
- It releases those two reads, observes exactly one new active admission, then yields once to let the second worker reach the active admission check before recording the count.
- It switches the same running scan to idle mode, releases the first new read, and observes exactly two newly admitted starts.
- It cancels and opens the permit gate before joining, then asserts the observed active and idle counts (`3` and `5`).
- No production code, scheduler logic, source media, or catalog data changed.

## Mutation check

Temporarily changed `admit_enrichment` from clamping the scheduler limit to `max(2)`, which ignores the active limit. The repaired test failed at the active-mode assertion:

```text
assertion failed: notifications.try_recv().is_err()
```

The mutation was restored before verification and commit.

## Verification

- Focused test, 20 consecutive runs: all passed.
- `cargo test -p photo-indexer`: 30 tests passed (18 progressive scan, 5 reconciliation, 7 scheduler priority); unit/doc test targets had 0 tests and passed.
- `cargo fmt --all --check`: passed.
- `cargo clippy -p photo-indexer --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.

## Commit

`b8afd1e` (`test(indexer): make admission transition cleanup safe`).

## Concerns

No known concerns. The native app was not launched; no source media or user catalog data was touched.

## Fix-round 1 evidence

The active-start race is removed by waiting for both initial idle reads to announce entry before changing to Active. The `PermitRelease` guard is drop-safe: it opens the gate to `usize::MAX` for all current and future reads, and the test explicitly opens it after cancellation so assertion failures or mutation failures cannot strand `spawn_blocking` readers.

With `admit_enrichment` temporarily mutated to use `max(2)` (ignoring the Active limit), the test exited promptly with the expected count failure:

```text
assertion `left == right` failed
left: 4
right: 3
```

The mutation was restored before the final verification.

Final fix-round verification: 20 consecutive focused runs passed; the complete `photo-indexer` suite passed 30 tests; fmt, Clippy with warnings denied, and `git diff --check` all passed.
