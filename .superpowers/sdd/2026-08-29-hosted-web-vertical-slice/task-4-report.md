# Task 4 report

## Files

Added the hosted selection HTTP routes, bounded SSE transport, shared request-target/query guards, and runtime-backed interaction lease scheduling. Selection summaries and walls resolve opaque stable IDs through `GalleryEngine`; event streams use scoped `SelectionEventSubscription::recv`, bounded replay/history recovery, monotonic IDs, heartbeats, and path-free errors.

## RED/GREEN

RED was observed with `cargo test -p photo-server --test gallery_api --test events_api`: the new create-selection test received 404 because the selection route did not exist. GREEN was observed after implementation with the focused server and runtime suites passing.

The lease regression covers active-client aggregation: an active client keeps the shared scheduler active while another client posts idle, and dropping the active subscription returns the scheduler to idle. Desktop bridge subscriptions remain outside hosted interaction leases so existing desktop scheduler semantics are preserved.

## Verification

- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-server --test gallery_api --test events_api` passed (4 tests).
- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-app-service --test hosted_runtime` passed (13 tests).
- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-app-service --test progressive_wall` passed (57 tests).
- `CARGO_INCREMENTAL=0 cargo test --offline --workspace` passed all workspace unit, integration, and doc tests.
- `cargo fmt --all -- --check` and `git diff --check` passed.

The full workspace run required `CARGO_INCREMENTAL=0` after reclaiming this worktree's generated `target` artifacts because the shared volume had only 170 MiB free. No source or tracked files were removed.

## Implementation commit

`c349e05` (`feat: expose selection scoped gallery events`).

## Concerns

The later derivative route must call the shared decoded identifier guard for its 512-byte ID limit and enforce its own 1–250 asset-list limit. Browser-side 10-second lease refresh remains a later adapter task; the server lease is 30 seconds and is refreshed by interaction requests.

## Fix round 1 evidence

RED was reproduced before the repair: the hostile future replay test returned the future watermark instead of the runtime head, and the hosted-runtime registry cleanup test timed out because the lease reaper retained the runtime while waiting. The missing HTTP regressions were then added first for future `u64::MAX` replay, header precedence, SSE framing, bounded bodies, route guards, child-folder classification, path-free errors, and unknown-client interaction.

GREEN after `93f77e3`:

- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-server --test gallery_api --test events_api` passed (7 gallery, 4 event tests).
- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-app-service --lib --tests` passed (92 unit tests and all app-service integration suites, including 14 hosted-runtime and 57 progressive-wall tests).
- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-server --tests` passed all server unit/integration suites.
- `CARGO_INCREMENTAL=0 cargo test --offline --workspace` passed all workspace unit, integration, benchmark-smoke, and doc tests.
- `cargo fmt --all -- --check` and `git diff --check` passed.

The reaper now has one weakly-owned resettable timer task per runtime, activates hosted demand when the stream opens, expires leases under paused Tokio time, preserves connected scope after expiry, and exits when the runtime drops. Desktop subscriptions use an internal origin enum and cannot be forged through a `desktop-*` client ID. Request guards run before gallery lookup and body buffering is bounded at 64 KiB with the fixed `invalidRequest` envelope.

Implementation commit: `93f77e3` (`fix: harden hosted gallery leases and transport`).

Remaining concerns are intentionally deferred to the task boundaries: the browser's 10-second refresh adapter and the derivative route's 512-byte/list limits belong to Tasks 5–6. No source-media writes were introduced.

## Fix round 2 evidence

The mandatory HTTP/runtime regression matrix was added test-first without changing release behavior. The additions cover successful wall paging and route cursor scope/direction rejection; normal `Last-Event-ID`, `afterEventId`, header precedence, retained-history resync and live recovery; deterministic 15-second heartbeat framing; bounded slow-consumer lag recovery; response-body drop cleanup; mixed HTTP scopes and aggregate demand; paused route lease expiry; query/body/identifier limits; public `desktop-*` Hosted behavior versus internal Desktop behavior; and multi-chunk request overflow with the fixed JSON envelope. Debug-only accessors expose the existing gallery runtime to these real-router lifecycle tests.

GREEN after round 2:

- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-server --lib --test gallery_api --test events_api` passed (1 API unit, 9 gallery, 10 event tests).
- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-app-service --lib --test hosted_runtime` passed (92 unit and 15 hosted-runtime tests).
- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-app-service --test progressive_wall` passed (57 tests).
- `CARGO_INCREMENTAL=0 cargo test --offline --workspace` passed all workspace unit, integration, benchmark-smoke, and doc tests.
- `cargo fmt --all -- --check` and `git diff --check` passed.

No round-2 production defect was exposed; only debug-only test hooks and regression coverage were added. The implementation baseline remains `93f77e3`; round-2 test coverage is committed separately.

## Fix round 3 evidence

Round 3 sharpened the regression assertions so each mandatory transport and guard case is discriminating rather than merely status-based. Valid selection and interaction JSON now carries ignored padding over the 64 KiB limit, including a multi-chunk body whose individual chunks are below the limit; the tests assert the fixed envelope and unchanged runtime state. Folder routes prove that benign eight-parameter requests succeed while nine parameters and an aggregate-over-8192 request are rejected, and the event suite rejects a parseable, zero-padded identifier whose textual form exceeds the fixed limit.

The HTTP matrix now publishes retained events before each replay request, proves after-only and header-only replay, verifies future-watermark recovery on the same stream and reconnect, drains fixture backlog before the deterministic heartbeat boundary, and keeps the broad route body connected through lease expiry to distinguish idle scheduler state from retained aggregate scope.

RED/GREEN: the first corrected heartbeat assertion went RED because a fixture scan event was still queued before virtual time advanced; draining that pre-existing event made the test target the heartbeat boundary and the focused rerun went GREEN. No production defect was exposed, so round 3 changes are regression tests only.

Verification for round 3:

- `cargo fmt --all && CARGO_INCREMENTAL=0 cargo test --offline -p photo-server --lib --test gallery_api --test events_api && CARGO_INCREMENTAL=0 cargo test --offline -p photo-app-service --lib --test hosted_runtime` passed (1 server unit, 9 gallery, 10 event, 92 app-service unit, and 15 hosted-runtime tests).
- `CARGO_INCREMENTAL=0 cargo test --offline -p photo-app-service --test progressive_wall` passed (57 tests).
- `CARGO_INCREMENTAL=0 cargo test --offline --workspace` passed all workspace unit, integration, benchmark-smoke, and doc tests.
- `cargo fmt --all -- --check` and `git diff --check` passed before the final commit gate.

The implementation baseline remains `93f77e3`; round-3 test coverage and this evidence update are committed separately.
