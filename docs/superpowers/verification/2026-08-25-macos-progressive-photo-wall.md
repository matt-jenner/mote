# macOS progressive photo wall verification

Date: 2026-08-26

Implementation commit verified: `318635423ee0fbd6399b8b3ce2ef2a7130198429`
Earlier evidence/documentation commit: `dafd36f` (documentation only)

## Result

The repaired Rust workspace, interface, desktop tests, WebKit suite, full
million-asset catalog benchmark, and unsigned macOS bundle all passed after a
fresh run. The six checked-in JPEG fixtures are unchanged. The earlier native
Tauri attempt launched with the `wall-demo` profile, but this environment could
not drive the native folder picker or capture the display. The screenshots in
this record therefore remain fallback renders made with headless WebKit from
the same six real JPEG fixtures. They show the intended states, but they are not
evidence that the native picker completed.

The final remediation adds safe legacy-catalog upgrades and folder-scoped
offline state, selection-scoped client convergence after rescan or channel lag,
and bounded concurrent derivative generation with exact cache-budget
serialization at the public cache boundary. Independent review and rereview
reported no remaining findings in each remediation batch.

## Task 2: thumbnail-first preview gate and native development profile

Task 2 is implemented on the `codex/progressive-photo-wall` worktree. The
service now keeps one shared, selection-fenced preview gate: recent requested
asset IDs are merged in stable de-duplicated order, a 50 ms quiet period is
debounced through one generation/task, and the task waits for an idle
interaction state, settled scan, zero queued or in-flight derivative work, and
more than the active-interaction background capacity before starting large
previews. Visible wall requests bump the gate generation before their work is
queued and remain scheduler-prioritised. Screen previews remain 4096 px and
non-durable; wall thumbnails remain 1024 px and durable.

Round 1 closes four lifecycle races. Wall requests use a
selection/generation-scoped cancellation-safe RAII guard, visible requests
fence the preview generation before catalog resolution, and stale queued screen
jobs are discarded before encoding. The single preview task parks on a shared
notification rather than polling; scan success, cancellation, unavailable and
error exits, interaction changes, queue transitions, wall completion, and
selection changes wake it. The corrected overlap test holds a wall thumbnail in
the second wave while interaction remains idle.

The focused RED/GREEN test is
`screen_previews_wait_for_all_overlapping_wall_thumbnail_waves`. The RED run
failed at the active/overlapping-wave assertion when the per-request preview
spawn was restored; the GREEN run passed after the shared gate was installed.
The app-service progressive-wall integration target passed all 28 tests, and
the workspace passed 182 tests across its integration and unit targets (51
app-service, 25 cache, 32 catalog, 16 core, 5 domain, 30 indexer, 12 metadata,
10 server, and 1 catalog-bench).

Fix-round RED evidence against reviewed base `9a6e4f7` was recorded for the
aborted-wall-request timeout, the queued-preview counter (`3` starts instead
of `2`), and the parked-gate retry counter (`2` checks instead of `1`). Their
GREEN runs passed after the RAII guard, generation fence, and notification
parking changes. The corrected wall-wave test is explicitly wall-gated with
idle interaction; it was already green on the reviewed base, so it is recorded
as a characterization rather than a manufactured RED.

The standalone desktop manifest contains only:

```toml
[profile.dev.package."*"]
opt-level = 3
```

The root workspace and release profiles were not changed. Cargo cannot select
the `photo-cache` integration target through the standalone desktop manifest
(`photo-cache` is a path dependency and is not a workspace member there), so
the profile-specific evidence is a same-workload before/after run of the real
desktop-manifest AppService protocol fixture, which scans a JPEG, generates a
wall derivative, and reads it through the desktop protocol. With the standalone
dependency profile temporarily disabled, the compile-warm command
took 1.12 s wall time (Cargo/setup 0.34 s, test body 0.73 s; user 0.84 s, sys
0.12 s). With the exact `opt-level = 3` profile restored, the same compile-warm
command took 0.38 s wall time (Cargo/setup 0.28 s, test body 0.05 s; user 0.16
s, sys 0.12 s). The disabled configuration was not committed. For context,
the same cache integration target took 4.38 s wall time in a warm root debug
build (4.19 s test body) and 0.41 s in root release (0.26 s test body). These
are machine observations, not hardware-neutral guarantees.

The final verification commands were all successful:

| Command | Result |
| --- | ---: |
| `cargo test -p photo-app-service --test progressive_wall` | 28 passed |
| `cargo test --workspace --all-features` | 182 passed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo fmt --all -- --check` | pass |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | 9 passed |
| `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings` | pass |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | pass |

The controlled source audit remained clean before and after verification:
`git diff --quiet -- apps/interface/public/demo-photos` passed, and no source
fixture path had a status entry. SHA-256 fixture hashes were:

```text
city.jpg     44faf249868e8d3ae81c0b532c460711526fbf17a66b82ee43ac805b3591cad1
coast.jpg    ad6ba90e75709487ab6b927dc3f2781042e31312a4b7bc4606cddd4c0c5c4394
forest.jpg   91c69c673ef96b8afff1c36da486de4dece98fcaeb178310d00b675f2e48a699
interior.jpg c8cc481a50bc60cdfb34e7e6ff2a0ac2e7350dd8f934afae91f3c19b0a2d3a05
mountain.jpg 79bcb7af04b68935b29b5e0a80afb17d1823dde94a5fdbc75e3d5adf7a6e62ae
portrait.jpg d1844a9747c7acfb36b10651ce79acff5c9d35b30a7b54323dbb171aed0bdfd0
```

## Fresh verification

All commands below exited 0.

| Area | Command | Result |
| --- | --- | ---: |
| Rust | `cargo fmt --all --check` | pass |
| Rust | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| Rust | `cargo test --workspace --all-features` | 177 tests passed |
| Rust | `cargo test -p catalog-bench --test benchmark_smoke` | 1 test passed |
| Rust | `cargo run -p catalog-bench --release -- --assets 1000000 --output target/catalog-benchmark.json` | 1,000,000 assets completed |
| Interface | `npm run check` | 36 files checked |
| Interface | `npm run typecheck` | pass |
| Interface | `npm test` | 58 tests passed |
| Interface | `npm run test:browser` | 29 tests passed in 2 files |
| Interface | `npm run --workspace @photo-viewer/interface build` | pass, 1,870 modules |
| Desktop | `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all --check` | pass |
| Desktop | `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` | pass |
| Desktop | `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | 9 tests passed |
| Desktop | `npm run desktop:build -- --bundles app` | unsigned `.app` created |

The browser suite passed in the approved run that permits its loopback test
server. The bundle is at the relative
path `apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`.

The million-asset report used SQLite 3.53.2 and produced a 645,300,224-byte
database. Insertion took 208,267.27 ms. The first 100-row page took 1.08 ms,
unavailable-asset counting took 125.52 ms, and whole-group eviction planning
took 4.48 ms. These are development-machine observations, not hardware-neutral
guarantees.

## Fallback visual evidence

These assets use the six real files under `apps/interface/public/demo-photos`.
The provisional frame leaves the second row colour-backed while the first row
has thumbnails. The refined frame fills the same geometry with thumbnails. The
GIF switches the direction label from oldest-first to newest-first.

![Provisional wall](assets/2026-08-25-wall-provisional.png)

![Refined wall](assets/2026-08-25-wall-refined.png)

![Oldest-first and newest-first](assets/2026-08-25-wall-sort.gif)

The fallback renderer used the six fixture bytes directly and did not modify
the fixture directory. It does not measure native first-paint or scroll timing.

## Native demonstration attempt

The command `PHOTO_VIEWER_PROFILE=wall-demo npm run desktop:dev` started Vite and
the Tauri desktop binary successfully. The native display capture command
returned `could not create image from display`. AppleScript could enumerate the
desktop process, but querying its window controls returned
`osascript is not allowed assistive access`. Without assistive access, I could
not click the native folder picker, switch sort direction, scroll, or create a
native screenshot or motion capture. I did not rename, delete, or move the
fixture directory to simulate offline mode.

The profile catalog inspection was read-only. It reported zero assets and zero
derivative rows because the picker could not be completed. Consequently,
`wall_thumbnail` and `screen_preview` rows were not available to report for
this native profile. The Rust cache tests still cover both tiers: the workspace
run included 12 cache-policy tests, 5 cache-budget tests, and 8 image-derivative
tests. A native cache-row capture remains an evidence gap for this environment.

First cached-paint and cached screen-preview timings are also not measured for
the same reason. The implementation targets remain 200 ms and 300 ms,
respectively, but this record makes no claim that the native run met them.

## Source read-only audit

The fixture audit ran `git diff --quiet -- apps/interface/public/demo-photos`
and checked that the fixture path had no status entries. Both checks passed.
The six JPEG SHA-1 values after the run were:

```text
city.jpg     23bf9dc7b767d80fa7fc552eb416f144855cc012
coast.jpg    7b1dfde062fbfc52b90a7e9c99b6cfab86e1ab5f
forest.jpg   fe9d20e37728e7ed20c261a44e2834938b8d8c54
interior.jpg d346c4a24dcdce7a2d8a224fe7911a950193c397
mountain.jpg 761f51655c42738085908b8eda813c3d93101090
portrait.jpg d441aa0dbecf293daf33ceb3fa41d6841262edbf
```

The production source path audit found writes confined to managed local state,
cache files, and SQLite. Source reconciliation tests include the explicit
offline-retention check. No production operation writes, renames, or deletes
source media.

## Checkpoint 3 limits

This checkpoint still accumulates mounted rows during a session. Row
virtualization, exact scroll anchoring, continuous row sizing, filename and
rating controls, multi-folder queries, and the immersive viewer remain
deferred to later checkpoints.

## Task 3: fresh verification of the thumbnail-first remediation

The verified implementation is `9cad2b0` (`fix: defer large previews behind
thumbnail work`). The Task 1 and Task 2 implementation range is
`d63cc9a..9cad2b0`; the Task 2 fix-round range is `a7992b8..9cad2b0`.
Task 2's warm profile comparison remains above: the desktop-manifest
AppService workload was 1.12 s with the dependency profile disabled and 0.38
s with `[profile.dev.package."*"] opt-level = 3`, while root cache context was
4.38 s debug and 0.41 s release. Those measurements were not rerun or
reinterpreted here.

The following fresh commands were run from the clean `codex/progressive-photo-wall`
worktree. `/usr/bin/time -p` `real` values are wall-clock observations on this
machine:

| Area | Exact command | Fresh result | real |
| --- | --- | ---: | ---: |
| Interface unit | `npm test` | 4 files, 64 passed | 0.87 s |
| Interface WebKit | `npm run test:browser` (local-port permission) | 2 files, 37 passed | 7.98 s |
| Interface typecheck | `npm run typecheck` | pass | 0.39 s |
| Biome | `npm run check` | 36 files checked, no fixes | 0.25 s |
| Interface build | `npm run --workspace @photo-viewer/interface build` | pass, 1,870 modules | 0.54 s |
| Progressive wall | `cargo test -p photo-app-service --test progressive_wall` | 28 passed, 0 failed | 4.58 s |
| Workspace | `cargo test --workspace --all-features` | 182 passed, 0 failed | 43.72 s |
| Root Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass | 0.45 s |
| Root format | `cargo fmt --all -- --check` | pass | 0.22 s |
| Benchmark smoke | `cargo test -p catalog-bench --test benchmark_smoke` | 1 passed, 0 failed | 2.18 s |
| Desktop tests | `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | 9 passed, 0 failed | 11.43 s |
| Desktop Clippy | `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` | pass | 2.42 s |
| Desktop format | `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | pass | 0.26 s |
| macOS bundle | `npm run desktop:build -- --bundles app` | 1 unsigned app bundle created | 23.72 s |

The browser command first attempted inside the restricted sandbox and failed
before test collection with `listen EPERM: operation not permitted
::1:63315`; the exact command was then rerun with local ephemeral-port
permission and produced the 37/37 result above. The bundle is at
`apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`. Its
executable is a Mach-O 64-bit arm64 binary. `codesign -dv --verbose=4`
reported an ad-hoc/linker signature, `TeamIdentifier=not set`, and
`Signature=adhoc`; it is not a Developer ID-signed or notarized artifact.
Strict deep verification reported `code has no resources but signature
indicates they must be present`, so the bundle is recorded as unsigned for
distribution purposes despite the linker ad-hoc signature.

## Task 3 source audit and native limitation

The production diff `d63cc9a..9cad2b0` changes only the app-service source,
desktop manifest, and progressive-wall tests. The filesystem-call scan found
no production source-path write, rename, move, copy, or delete operation. The
only `create_dir_all` hits in the changed app-service file are test fixture
setup under `cfg(test)`. `git diff --quiet -- apps/interface/public/demo-photos`
returned exit 0 both before and after verification, and the fixture directory
had no status entries. The before/after SHA-256 values are identical:

```text
city.jpg      44faf249868e8d3ae81c0b532c460711526fbf17a66b82ee43ac805b3591cad1
coast.jpg     ad6ba90e75709487ab6b927dc3f2781042e31312a4b7bc4606cddd4c0c5c4394
forest.jpg    91c69c673ef96b8afff1c36da486de4dece98fcaeb178310d00b675f2e48a699
interior.jpg c8cc481a50bc60cdfb34e7e6ff2a0ac2e7350dd8f934afae91f3c19b0a2d3a05
mountain.jpg 79bcb7af04b68935b29b5e0a80afb17d1823dde94a5fdbc75e3d5adf7a6e62ae
portrait.jpg d1844a9747c7acfb36b10651ce79acff5c9d35b30a7b54323dbb171aed0bdfd0
```

A fresh named profile, `task3-clean-smoke`, was launched with
`PHOTO_VIEWER_PROFILE=task3-clean-smoke npm run desktop:dev`. After granting
local-port permission, Vite and the Tauri binary started successfully. Native
interaction remained unavailable: `screencapture -x` returned `could not
create image from display`, and the System Events query returned `osascript is
not allowed assistive access. (-1728)`. The folder picker could not be
completed, so no native geometry-paint, first-thumbnail, refined-viewport,
contact-sheet, preview-gating, pending-sort, or progress-transition timings
are claimed. The dev process was stopped after the attempt; no desktop app is
left running. Native interaction remains user-verified rather than
automation-verified in this environment.
