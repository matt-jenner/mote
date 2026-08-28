# Exceptional final collection-driver fix report

Date: 2026-08-29
Branch: `codex/viewer-zoom-pan`
Status: complete

## Scope

This fix closes the final collection-driver rereview findings on top of
`3336390`:

- collection intent now retains the latest `SelectionToken` epoch and rejects
  delayed older calls, while same-selection calls still OR `full_group`;
- admission and lifecycle tracking guards are acquired before
  `tokio::spawn`, and a never-polled future abandons its pending intent;
- the driver atomically takes work or releases admission before its exit gate,
  while the guard drop order registers any successor before the old tracker can
  reach zero;
- the stress fixture sends 1,000 actual recent-intent triggers over 500
  distinct synthetic `AssetId`s and asserts the exact 250-item recency window.

No GUI, merge, push, frontend, Tauri, plan, specification, or ledger files
were touched.

## RED/GREEN evidence

The regression tests were added before the implementation. The first focused
compile run failed at the missing production boundaries:

```text
error[E0599]: no method named `expect` found for unit type `()`
  --> crates/app-service/src/derivatives.rs:3412
error[E0599]: no method named `install_collection_driver_post_take_test_gate`
  --> crates/app-service/src/derivatives.rs:3497
```

After implementing the synchronous guard admission, monotonic selection
control, atomic take-or-release handoff, and post-take test gate, the focused
suite passed:

```text
cargo test -p photo-app-service collection_driver_ --lib -- --test-threads=1
7 passed; 0 failed
```

That focused suite was repeated 100 times after the final source cleanup; all
100 runs completed with 7/7 tests passing. It includes the pre-poll abort and
later re-admission regression, newer-selection/older-resume tests, the
post-take successor/quiescence gate, and the 500-distinct-ID stress case.

## Verification

```text
cargo test -p photo-app-service --lib -- --test-threads=1
70 passed; 0 failed

cargo test -p photo-app-service --test progressive_wall -- --test-threads=1
54 passed; 0 failed

cargo test -p photo-app-service --test task7_source_safety -- --test-threads=1
1 passed; 0 failed

cargo test -p photo-app-service --lib --release -- --test-threads=1
53 passed; 0 failed

cargo check --workspace --release
exit 0

cargo test --workspace --all-targets -- --test-threads=1
exit 0

cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
exit 0

cargo fmt --all -- --check
exit 0

git diff --check
exit 0
```

The release test run was executed with the debug-only collection tests
excluded, confirming the production build remains warning-free. The Task 7
test performed the real service restart/source-safety sequence and passed
after the final implementation commit.

## Bounds, liveness, and source safety

The collection control retains one pending selection/full-group intent and one
latest selection token; it does not retain request vectors. The coordinator
continues to own the deduplicated, capped recent window. The stress test
observed a maximum of one admitted collection driver during Active interaction,
an exact recent length of 250 after 500 distinct IDs cycled twice, and clean
quiescence after Idle. The post-take gate proves a request arriving after the
old driver has released admission registers a successor while the old task is
still tracked, so quiescence cannot report zero between handoff steps.

The source audit compares implementation base `3336390` with code head
`e634126`. It lists only the expected Rust files:

```text
M	crates/app-service/src/derivatives.rs
M	crates/app-service/src/service.rs
```

The exact name-status output SHA-256 is
`4d1900107842ab4cd6e1c0b4fbb28404c0ff07df4cae4f73917df7357583ece9`.
The normalized added-line filesystem/source-path audit returned zero matches;
its output SHA-256 is
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The Task 7 safety harness remains green, and no source-media write, delete,
rename, move, or copy operation was introduced.

## Commit

Implementation and deterministic regressions:

`e634126` (`fix: serialize collection driver handoff`)

This report is the documentation follow-up to that implementation commit.
