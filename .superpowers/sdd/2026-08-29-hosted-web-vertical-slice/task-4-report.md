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
