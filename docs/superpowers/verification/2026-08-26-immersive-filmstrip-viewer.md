# Immersive filmstrip viewer verification

Date: 2026-08-26

Status: verification complete for the interface, Rust workspace, and desktop
build. The coordinator-owned `wall-demo` native app was left running and was
not stopped or replaced.

## Revision and changed files

- Base: `6ce0e798e761d4f70a74ba5046c74c7b3325eb0b`
- Evidence commit: `2579c94` (`test: verify immersive filmstrip viewer`)
- Modified: `README.md`
- Modified: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Added: this verification document
- Deleted: `.superpowers/sdd/2026-08-26-immersive-filmstrip-viewer/task-3-report.md`
- Deleted: `.superpowers/sdd/2026-08-26-immersive-filmstrip-viewer/task-4-report.md`

The two deleted files were tracked process reports. Their useful evidence is
consolidated here. The ignored SDD workspace remains in place. Generated
Vitest screenshots and attachments were not added to Git.

## RED and GREEN evidence

The new browser assertions were written before their focused verification. The
first focused WebKit run was blocked before test execution by the sandbox
listener restriction (`listen EPERM` on `::1`). The permitted red run then
failed in the new boundary, phone-timer, and cached-neighbour assertions. The
boundary fixture was corrected to use a two-asset sequence, and the phone
assertion was kept to measured target sizes because the existing desktop timer
tests already prove hidden controls remove focusability. The focused run then
completed with 5 selected tests passed.

The final `PhotoViewer.browser.test.tsx` coverage now checks:

- Axe serious/critical violations with the viewer open, with the information
  drawer open, at both navigation boundaries, and after portrait-to-landscape
  phone rotation.
- `aria-current`, disabled previous/next state, 44px phone targets, and the
  existing hidden-control `tabIndex=-1` behavior.
- Ready thumbnail and screen-preview URLs after `sourceUnavailable`, including
  cached-neighbour navigation. Assertions only see derivative URLs and verify
  no native source identity is exposed.
- `data-large-preview-unavailable`, the information-drawer marker, and zero
  reduced-motion transition duration.

The complete browser suite is GREEN: 3 files, 79 tests passed.

## Exact command results

| Command | Result |
|---|---|
| `npm test` | PASS, 10 files, 89 tests |
| `npm run test:browser` | PASS, 3 files, 79 tests |
| `npm run typecheck` | PASS, `tsc -b --pretty false` |
| `npm run check` | PASS, Biome checked 55 files |
| `npm run --workspace @photo-viewer/interface build` | PASS, 1882 modules; JS 290.28 kB, CSS 16.89 kB |
| `cargo test --workspace --all-features` | PASS, 190 passed, 0 failed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | PASS, 11 passed, 0 failed |
| `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings` | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | PASS |
| `cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-viewer.json` | PASS, 10,000 assets; report written |
| `npm run desktop:build -- --bundles app` | PASS, one unsigned macOS app bundle |
| `git diff --check` | PASS before evidence commit |

The benchmark report is at
`target/catalog-benchmark-viewer.json`: SQLite 3.53.2, database 6,602,752
bytes, first page 100 rows, insert 810.23 ms, first page 0.23 ms,
unavailable count 0.96 ms, eviction plan 0.06 ms.

## Source-media invariant

Only the controlled checked-in fixture directory was hashed:
`apps/interface/public/demo-photos`. Its six-file SHA-256 manifest was:

`44faf249868e8d3ae81c0b532c460711526fbf17a66b82ee43ac805b3591cad1  city.jpg`

`ad6ba90e75709487ab6b927dc3f2781042e31312a4b7bc4606cddd4c0c5c4394  coast.jpg`

`91c69c673ef96b8afff1c36da486de4dece98fcaeb178310d00b675f2e48a699  forest.jpg`

`c8cc481a50bc60cdfb34e7e6ff2a0ac2e7350dd8f934afae91f3c19b0a2d3a05  interior.jpg`

`79bcb7af04b68935b29b5e0a80afb17d1823dde94a5fdbc75e3d5adf7a6e62ae  mountain.jpg`

`d1844a9747c7acfb36b10651ce79acff5c9d35b30a7b54323dbb171aed0bdfd0  portrait.jpg`

Aggregate manifest hash before final audit/build follow-up:
`a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6`.
Aggregate manifest hash after all relevant checks is the same
`a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6`.

The production diff from the Task 7 base contains no source-media write,
rename, move, copy, rating, tagging, or delete operation. The viewer reads
only host-neutral catalog and derivative references. SQLite and managed cache
locations remain the only intended writable locations. No user-selected path
was recorded.

## Fix round 1

The workspace Clippy finding was fixed with the prescribed behavior-preserving
let-chain rewrite in `crates/app-service/src/derivatives.rs`. The controlled
offline browser harness now primes opaque current and neighbour derivative
URLs, rejects uncached access after `sourceUnavailable`, and injects the
internal sentinel `file:///private/source/secret.jpg`; the viewer never renders
that sentinel. The stage itself remains on the cached screen-preview layer for
the current photo and a decoded cached wall-thumbnail layer after navigating to
the neighbour. The phone test also measures the Info drawer close target.

Fresh fix-round results: focused offline/phone browser tests 2 passed; full
browser suite 3 files and 79 passed; unit suite 10 files and 89 passed; Rust
workspace 190 passed; workspace Clippy and fmt passed; desktop tests 11 passed,
desktop Clippy and fmt passed; typecheck, Biome, interface build, desktop app
build, and `git diff --check` passed. The unsigned bundle remains at
`apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`.

## Native evidence and limits

The exact desktop build created:

`apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`

This environment independently established the build and desktop Rust tests.
It did not independently observe macOS clicks, swipes, window resizing,
orientation changes, source disconnection, or return-to-wall focus. The
coordinator-owned native session using profile `wall-demo` was already running;
it was not stopped, relaunched, or interfered with. Those exploratory claims
remain open for the user's acceptance check.

The approved Task 7 rotation proof preserves the same ready decoded
screen-preview DOM node through rotation. It does not describe a wall-thumbnail
layer replacement. Zoom, pan, folder recovery, source editing, backend changes,
signing, notarization, and merge/push remain outside this slice.

## Self-review

The browser tests use controlled in-memory assets and path-free derivative
references. Generated screenshots and attachment directories remain untracked.
README now gives the viewer controls and a clean-profile/native demonstration
command. The only verification concern is the pre-existing workspace Clippy
failure listed above. The app state at handoff is the coordinator's running
`wall-demo` session, unchanged.
