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
- The test starts the scan with two workers, switches the shared scheduler to active mode, and observes exactly one start with no second notification.
- It switches the same running scan to idle mode, releases the first read, and observes exactly two newly admitted starts.
- It cancels and releases the remaining blocked reads before joining, then asserts the observed active and idle counts (`1` and `3`).
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

`df43448` (`test(indexer): gate admission mode transitions`).

## Concerns

No known concerns. The native app was not launched; no source media or user catalog data was touched.
