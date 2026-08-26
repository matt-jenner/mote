# Task 2 report: defer large previews and optimise native development image work

Date: 2026-08-26
Status: complete
Branch: `codex/progressive-photo-wall`
Commit: `fix: defer large previews behind thumbnail work` (commit hash is recorded by the final git commit)

## Files

- `crates/app-service/src/derivatives.rs`
- `crates/app-service/src/service.rs`
- `crates/app-service/tests/progressive_wall.rs`
- `apps/desktop/src-tauri/Cargo.toml`
- `docs/superpowers/verification/2026-08-25-macos-progressive-photo-wall.md`
- this report

No interface files were changed. Source media was not modified.

## RED/GREEN evidence

The focused command was:

```text
cargo test -p photo-app-service --test progressive_wall screen_previews_wait_for_all_overlapping_wall_thumbnail_waves -- --exact
```

RED used the base per-request preview-start behavior with the deterministic
screen-preview boundary held during active interaction and overlapping wall
requests. It exited 101 at the assertion that the screen gate must not be
entered while the wall waves/interaction were active:

```text
thread 'screen_previews_wait_for_all_overlapping_wall_thumbnail_waves' ... FAILED
screen previews must wait while interaction and overlapping wall work are active
```

GREEN with the shared preview gate exited 0:

```text
running 1 test
test screen_previews_wait_for_all_overlapping_wall_thumbnail_waves ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 24 filtered out
```

The existing active-interaction regression was updated to require all large
preview work to wait for idle, then verified independently:

```text
test returning_to_idle_resumes_remaining_screen_preview_prefetch ... ok
```

## Gate lifecycle and race reasoning

`ServiceState.recent_derivative_ids` now merges requested IDs in stable,
deduplicated order and is cleared on selection change. Each wall request bumps
the preview generation and records an in-flight wall-request marker before
queueing work; it decrements the marker after the wall wave completes and then
wakes the gate.

The service owns one `PreviewGateState` and one wake notification. Scheduling
merges the stable recent IDs, increments the generation, and starts a task only
when no gate task is already running. The task waits for one 50 ms quiet period
and then repeatedly checks:

1. the generation is still current and the active selection token is still
   active;
2. no scan is active;
3. the derivative queue is empty, including queued and in-flight wall work;
4. the scheduler has more than the one active-interaction background permit;
5. no wall request is still in flight.

If a gate is closed, the same task waits on the wake notification or a bounded
25 ms retry. Returning to idle, scan settlement, wall completion, and a new
request all wake or invalidate the same task. A new visible request bumps the
generation before it queues wall work, so a pending preview start exits and the
visible scheduler priority remains ahead of queued preview work. The
`task_running` state is changed under the same mutex as the pending IDs, which
closes the empty-task/new-request lost-wake race.

Once all gates pass, recent IDs are resolved and generated first with
`NearViewport` priority; the remaining group is paginated with `IdleLibrary`
priority. Selection checks remain before preview resolution and before the
existing screen-preview cache-budget/write/catalog transaction. The existing
cache-root transaction lock and lock ordering are unchanged.

## Verification counts

The complete Task 2 verification commands all exited 0:

| Command | Exact result |
| --- | ---: |
| `cargo test -p photo-app-service --test progressive_wall` | 25 passed |
| `cargo test --workspace --all-features` | 178 passed; 0 failed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo fmt --all -- --check` | pass |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | 9 passed; 0 failed |
| `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings` | pass |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | pass |

The workspace total is 178 passing tests: catalog bench 1, app-service 47,
cache 25, catalog 32, core 16, domain 5, indexer 30, metadata 12, and server
10. Desktop has 9 passing unit tests.

## Warm timing methodology and results

Each command was run once to account for any compilation, then repeated after
the target was warm. `/usr/bin/time -p` wall/user/sys values were recorded;
test-body times are taken from Cargo's test summary and are reported separately
from compilation/setup.

The direct standalone-manifest cache selection was attempted and rejected by
Cargo because `photo-cache` is a path dependency requiring dev-dependencies
and is not a member of the standalone workspace:

```text
error: package `photo-cache` cannot be tested because it requires dev-dependencies and is not a member of the workspace
```

Root cache workload context:

| Workload | Cargo/setup | Test body | `/usr/bin/time -p` real |
| --- | ---: | ---: | ---: |
| `cargo test -p photo-cache --test image_derivative` (debug, first timed run) | 0.12 s finished profile | 4.19 s | 4.58 s |
| same command, compile-warm debug | 0.07 s finished profile | 4.19 s | 4.30 s |
| `cargo test --release -p photo-cache --test image_derivative` | 0.14 s finished profile | 0.26 s | 0.46 s |

The profile-specific real desktop-manifest/AppService execution was
`protocol::tests::derivative_protocol_returns_jpeg_with_cache_security_headers`.
It scans a controlled JPEG, generates a wall thumbnail through AppService, and
reads the derivative through the desktop protocol:

| Run | Cargo/setup | Test body | `/usr/bin/time -p` real |
| --- | ---: | ---: | ---: |
| after source rebuild under standalone profile | 11.10 s | 0.05 s | 12.24 s |
| compile-warm standalone profile | 0.50 s finished profile | 0.05 s | 0.62 s |

The standalone manifest now has exactly `[profile.dev.package."*"] opt-level =
3`; root debug and release profiles were not changed. The desktop AppService
execution is the profile-specific evidence because Cargo cannot directly run
the path dependency's cache integration target from that manifest.

## Fixture hashes and source-write audit

Before and after verification, `git diff --quiet --
apps/interface/public/demo-photos` passed and the fixture directory had no
status entries. SHA-256 hashes after verification (identical to the before
snapshot) are:

```text
city.jpg     44faf249868e8d3ae81c0b532c460711526fbf17a66b82ee43ac805b3591cad1
coast.jpg    ad6ba90e75709487ab6b927dc3f2781042e31312a4b7bc4606cddd4c0c5c4394
forest.jpg   91c69c673ef96b8afff1c36da486de4dece98fcaeb178310d00b675f2e48a699
interior.jpg c8cc481a50bc60cdfb34e7e6ff2a0ac2e7350dd8f934afae91f3c19b0a2d3a05
mountain.jpg 79bcb7af04b68935b29b5e0a80afb17d1823dde94a5fdbc75e3d5adf7a6e62ae
portrait.jpg d1844a9747c7acfb36b10651ce79acff5c9d35b30a7b54323dbb171aed0bdfd0
```

The progressive-wall tests use temporary fixtures under `tempfile`; those
fixtures are deleted with their temporary directories. Production writes remain
limited to managed local state, SQLite, and the cache. No source file is
written, renamed, moved, copied, or deleted.

## Concerns

- The native display/picker could not be driven in this environment, so there
  is no native screenshot or first-paint measurement.
- The direct `photo-cache` integration selection from the desktop manifest is
  unavailable; the report therefore uses the real desktop AppService protocol
  fixture for standalone-profile evidence and labels root cache timings as
  context.
- The deterministic integration boundary is debug-only and is not present in
  release builds. It exists solely to make the overlapping-wave behavior test
  deterministic; it has no production cache or source side effect.
