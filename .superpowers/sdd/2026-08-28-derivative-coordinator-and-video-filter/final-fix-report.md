# Derivative collection-driver final-fix report

Date: 2026-08-28
Branch: `codex/viewer-zoom-pan`
Status: complete

## Scope

This final whole-branch fix replaces per-trigger delayed collection-driver
tasks with one service-owned coalescing admission. Recent IDs remain in the
coordinator's bounded 250-item window; pending intent stores only the active
selection and a monotonic full-group bit. Interaction changes and derivative
requests merge intent and wake the existing driver. Admission remains held
until the task guard drops, including the exit handoff, cancellation, and
panic paths.

## RED/GREEN evidence

The new exit-handoff test was written before the implementation. Its first
compile run was red because the deterministic exit-gate API did not yet
exist:

```text
error[E0599]: no method named `install_collection_driver_exit_test_gate`
found for struct `AppService`
```

After implementation, the focused tests passed:

```text
cargo test -p photo-app-service --lib \
  derivatives::tests::collection_driver_triggers_coalesce_under_active_interaction \
  -- --exact --test-threads=1
test result: ok; 1 passed; 0 failed

cargo test -p photo-app-service --lib \
  derivatives::tests::collection_driver_merges_full_group_intent_at_exit_without_lost_wake \
  -- --exact --test-threads=1
test result: ok; 1 passed; 0 failed

focused coalescing and exit-handoff tests repeated 100 times each
coalesce_rc=0 exit_rc=0
```

The stress test issues 1,000 alternating Active interaction and real-asset
recent-intent triggers, records a maximum of one admitted collection driver,
and asserts the recent window stays at most 250. It then returns to Idle and
awaits driver quiescence. The gated handoff test admits a recent-only driver,
arrives at its exit boundary, merges a full-group request, and proves the
thumbnail/preview collection reaches `Complete`.

## Verification

```text
cargo test -p photo-app-service --lib -- --test-threads=1
test result: ok; 66 passed; 0 failed

cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
test result: ok; 54 passed; 0 failed

cargo test -p photo-catalog --test wall_query -- --test-threads=1
test result: ok; 19 passed; 0 failed

cargo test -p photo-catalog --test catalog_round_trip -- --test-threads=1
test result: ok; 12 passed; 0 failed

cargo test -p photo-app-service --test task7_source_safety -- --test-threads=1
test result: ok; 1 passed; 0 failed

cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
Finished successfully

cargo fmt --all -- --check
exit=0

git diff --check
exit=0

cargo check --workspace --release
Finished successfully

cargo test -p photo-app-service --lib --release -- --test-threads=1
test result: ok; 53 passed; 0 failed

cargo test --workspace --all-targets -- --test-threads=1
workspace_exit=0
```

## Boundedness, liveness, and source safety

The control state retains one pending selection/full-group intent and no
per-request ID vector. The existing coordinator owns the deduplicated recent
window and caps it at 250. The collection-driver tracker reports one active
driver throughout the 1,000-trigger stress, and the exit gate proves a
request arriving while the driver is deciding to finish is consumed by the
same admission or its serialized successor. No state lock is held across
collection I/O or await points; the test-only exit gate is the sole explicit
interleaving boundary.

The exact source audit compares base
`6d4296b52429d8aa5806492d3e25c8820efad02d` with implementation head
`aa7a268e2f7fc86d27d682d94f54b049dfdf6780`. It reports only the two expected
Rust source files, and the existing filesystem/source-path audit finds zero
added-line matches. The name-status output SHA-256 is
`4d1900107842ab4cd6e1c0b4fbb28404c0ff07df4cae4f73917df7357583ece9`; the
zero-match output SHA-256 is
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The final Task 7 source-safety snapshot records the same six demo files,
sizes, mtimes, and content digests before and after the restart sequence.
The before and after snapshot output SHA-256 is
`7d44d5aa0366d1e2dc63d09486637b3790db056e3fe071edc9cec8055c96c979`.
No GUI process was launched, stopped, merged, or pushed by this fix.

## Commit

Implementation and deterministic tests:
`aa7a268e2f7fc86d27d682d94f54b049dfdf6780` (`fix: coalesce collection driver wakeups`).

The report and verification-document follow-up is intentionally a separate
documentation commit; its SHA is recorded by the post-commit handoff.
