# macOS progressive photo wall verification

Date: 2026-08-25

Implementation commit verified: `44ca7605c1b3f1d37c7e67c82c8289c1b24d1984`
Evidence/documentation commit: `dafd36f` (documentation only)

## Result

The Rust workspace, interface, desktop tests, and unsigned macOS bundle all
passed after a fresh run. The six checked-in JPEG fixtures are unchanged. The
native Tauri app launched with the `wall-demo` profile, but this session could
not drive the native folder picker or capture the display. The screenshots in
this record are therefore a fallback render made with headless WebKit from the
same six real JPEG fixtures. They show the intended states, but they are not
evidence that the native picker completed.

The fresh root run also included five small, behavior-preserving fixes required
by the current Rust toolchain: the benchmark now initializes
`folder_group_id`, Clippy-compatible forms replace nested conditionals and
manual arithmetic, and the existing nine-argument preview API has a scoped
Clippy allowance. These edits are included in the worktree for review.

## Fresh verification

All commands below exited 0.

| Area | Command | Result | Wall time |
| --- | --- | ---: | ---: |
| Rust | `cargo fmt --all --check` | pass | 0.17 s |
| Rust | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass | 2.83 s |
| Rust | `cargo test --workspace --all-features` | 162 tests passed | 41.21 s |
| Rust | `cargo test -p catalog-bench --test benchmark_smoke` | 1 test passed | 6.61 s |
| Interface | `npm run check` | 36 files checked | 0.23 s |
| Interface | `npm run typecheck` | pass | 0.61 s |
| Interface | `npm test` | 43 tests passed | 0.80 s |
| Interface | `npm run test:browser` | 24 tests passed in 2 files | 4.14 s |
| Interface | `npm run --workspace @photo-viewer/interface build` | pass, 1,870 modules | 0.59 s |
| Desktop | `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all --check` | pass | 0.37 s |
| Desktop | `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` | pass | 2.86 s |
| Desktop | `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | 7 tests passed | 7.95 s |
| Desktop | `npm run desktop:build -- --bundles app` | unsigned `.app` created | 66.95 s |

The browser suite first hit the sandbox's loopback bind restriction. The same
command passed in the approved unsandboxed run. The bundle is at the relative
path `apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`.

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
run included 12 cache-policy tests, 5 cache-budget tests, and 7 image-derivative
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
