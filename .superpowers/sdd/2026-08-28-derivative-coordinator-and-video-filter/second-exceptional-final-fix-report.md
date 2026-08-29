# Second exceptional collection-driver fix report

Date: 2026-08-29
Branch: `codex/viewer-zoom-pan`
Base: `9f75a39`
Status: complete

## Scope

This fix replaces the paired collection-driver admission/task guards with a
single lifecycle owner. The owner carries the admission identity and, in test
and debug builds, its embedded derivative-task tracker guard. The owner is
constructed synchronously before `tokio::spawn`.

- Normal `take_pending_or_release` is an atomic control transition. A release
  disarms the owner, so its later `Drop` cannot release admission or spawn a
  duplicate successor.
- Owner `Drop` compares the exact owner identity, preserves a concurrent newer
  pending intent, and registers any successor before the embedded tracker guard
  can decrement or wake quiescence. A stale owner cannot clear a successor.
- The request path acquires a candidate tracker before publishing a new
  admission, closes the request-to-first-poll quiescence gap, and transfers
  that guard into the owner before spawning.
- Selection epochs remain monotonic; same-selection requests OR
  `full_group`, while delayed older calls are ignored. Runtime-unavailable
  admission is abandoned without rescheduling.
- The coordinator retains one pending collection intent and the existing
  bounded, deduplicated recent window; no collection-driver task retains a
  per-request ID vector.

No GUI, merge, push, frontend, Tauri, plan, specification, ledger, or source
media files were touched.

## RED/GREEN evidence

The second-wave stale-selection, pre-poll cancellation, post-release,
quiescence, and bounded stress regressions were added before the production
owner implementation. The first focused compile was intentionally red at the
old control API boundary, including:

```text
error[E0599]: no method named `expect` found for unit type `()`
error[E0599]: no method named `install_collection_driver_post_take_test_gate`
```

After the lifecycle-owner implementation and the panic-path regression, the
focused collection-driver suite passed:

```text
cargo test -p photo-app-service collection_driver_ --lib -- --test-threads=1
10 passed; 0 failed
```

That exact focused suite was repeated 100 times; every run completed with
10/10 tests passing. It covers monotonic newer-selection transfer, pre-poll
abort, cancellation and panic transfer of newer full-group intent, post-take
successor admission, quiescence ordering, exit lost-wake behavior, and the
1,000-trigger/500-distinct-ID bounded stress path.

## Verification

```text
cargo test -p photo-app-service --lib -- --test-threads=1
73 passed; 0 failed

cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
54 passed; 0 failed

cargo test -p photo-app-service --test task7_source_safety -- --test-threads=1
1 passed; 0 failed

cargo test -p photo-app-service --lib --release -- --test-threads=1
53 passed; 0 failed

cargo test --workspace --all-targets -- --test-threads=1
exit 0

cargo check --workspace --release
exit 0

cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
exit 0

cargo fmt --all -- --check
exit 0

git diff --check
exit 0
```

The workspace and Task 7 commands were run at code head `9812398` after the
panic-path test was added. Task 7 completed its real restart and source-safety
harness without source-tree changes.

## Bounds, liveness, and source safety

The Active-interaction stress test sends 1,000 actual recent-intent triggers
using 500 distinct synthetic `AssetId`s. It observes a maximum of one admitted
collection driver, an exact recent length of 250 with the expected recency
window, then returns to Idle and reaches driver quiescence. The post-take gate
admits one successor while the old task remains tracked and repeated requests
never create a second admitted owner or duplicate successor. The quiescence
waiter regression proves owner admission cleanup and successor registration
precede tracker zero, including never-polled abort and panic unwinding.

The exact source audit compares `9f75a39` with code head `9812398`:

```text
M	crates/app-service/src/derivatives.rs
M	crates/app-service/src/service.rs
```

The name-status output SHA-256 is
`4d1900107842ab4cd6e1c0b4fbb28404c0ff07df4cae4f73917df7357583ece9`.
The normalized added-line filesystem/source-path audit returned zero matches;
its output SHA-256 is
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
No source-path write, delete, rename, move, or copy operation was introduced.

## Commits

Implementation and deterministic lifecycle regressions:

- `b8881b6` (`fix: unify collection driver lifecycle ownership`)
- `9812398` (`test: cover collection driver panic handoff`)

The report is committed separately as a documentation follow-up.
