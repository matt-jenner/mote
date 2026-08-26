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
