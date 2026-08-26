# Immersive filmstrip viewer final-fix report

Status: complete. All 13 final-review findings are addressed in one logical
implementation commit.

Base: `77cad56c3d0c055c0e2ceb2cda27b37ec846f5d7`

Head: `f33a10b14ae17043dd5232b73a5a029a5694b16d` (`fix: close immersive viewer review gaps`)

The required full source audit range is `0ccfa7c..HEAD`. The report and the
durable verification update are documentation follow-up to the implementation
commit.

## Finding-by-finding changes and tests

1. Current-tile return: `AppShell` records the viewer's current asset at close,
   preserves the opening wall scroll position, focuses/highlights that current
   tile, and falls back to the programmatically focusable wall region when the
   tile is absent. Browser coverage: “returns focus and highlight to the asset
   viewed when closing” and “closes when the current asset disappears during an
   open viewer”.

2. Focus stability: entry focus is separated from the stable Escape/listener
   wiring; control focus is retained across selection, drawer, timer, and
   preview renders. Browser coverage: “does not steal focus from viewer
   controls when selection or drawer state rerenders”.

3. Bounded preview failure: preview requests are keyed by asset, generation,
   and derivative URL, use three total attempts with 100 ms and 250 ms retry
   delays, surface a permanent rejection immediately, and expose the drawer
   unavailable message. Browser coverage: “surfaces backend preview rejection
   immediately and caps retries”, “retries a rejected screen-preview request”,
   “marks a permanently rejected preview unavailable and stops retrying”, and
   “promotes a near-viewport request to visible after switching assets”.

4. Stage fit: a padding-free measurement element is used for the drawable box;
   `drawableViewerBox` subtracts safe padding and `fitViewerFrame` preserves
   aspect ratio within the available bounds. Unit coverage: two geometry tests
   for safe padding and desktop/phone bounds.

5. Base/overlay failure isolation: stage failure state is fenced by base URL
   and screen-preview asset/generation/decode token, so a broken asset cannot
   poison a valid neighbour. Browser coverage: “advances from a broken screen
   preview to a decoding wall thumbnail” and “isolates base and screen failures
   by derivative URL and asset generation”.

6. Complete chrome auto-hide: Back, Previous, Next, Info, and filmstrip are
   included in the chrome policy; hidden controls leave tab order, while an
   active control remains visible until focus leaves. Open drawers retain Info
   and Close. Browser coverage: “removes the full chrome from tab order when
   controls hide”, “cycles visible drawer controls while primary controls are
   hidden”, and the mouse/touch chrome tests.

7. Drawer safe areas: the drawer width and all edge padding account for right,
   left, top, and bottom safe insets, including rotated landscape values.
   Browser coverage: “keeps the information drawer close target inside rotated
   safe areas”.

8. Viewer interaction and scheduler: the existing wall interaction API is
   passed into the overlay; keyboard, button, filmstrip, and touch activity are
   reported. The idle neighbour group is deferred through idle scheduling or a
   delayed interaction-state recheck rather than a one-millisecond fallback.
   Browser coverage: “reports keyboard, button, filmstrip, and touch activity
   to the wall” and “defers the idle neighbour group while interaction remains
   active”.

9. Full-range source audit: the production diff was audited over
   `0ccfa7c..HEAD`, and the controlled derivative test generates only in a
   temporary managed cache while asserting the checked-in source bytes and
   modification time are unchanged. No selected-folder path is recorded.

10. Normal-motion crossfade: the `browser-motion` Vitest WebKit project uses
    `prefers-reduced-motion: no-preference`. Its crossfade test proves the
    thumbnail stays visible while the preview decodes, opacity transitions,
    fitted bounds remain unchanged, and the preview becomes ready. The existing
    reduced-motion assertions remain in the regular browser project.

11. Filmstrip window: `ViewerFilmstrip` memoizes its visible asset slice and
    only reruns missing-thumbnail work when that slice or its request callback
    changes.

12. Pointer routing: active gestures retain `pointerType`; only touch taps
    toggle chrome and only touch swipes navigate. Mouse clicks and drags do not
    enter touch gesture routing. Browser coverage: “navigates with a horizontal
    touch swipe and toggles chrome on a tap” and “keeps mouse drags and clicks
    out of touch gesture routing”.

13. Indexed video policy: video tiles are poster-only and non-openable, with a
    restrained accessible cue. No video playback or transcoding was added.
    Browser coverage: “keeps indexed videos poster-only and non-openable”.

The approved ruling is recorded in `README.md` and the binding spec: an asset
without a usable cached derivative does not open this viewer because
`PhotoService` has no locate-folder capability. Desktop locate/reconnect is a
named follow-up; the hosted web surface does not offer local folder selection,
so there is no in-viewer recovery path in this slice.

## RED/GREEN evidence

- Viewer geometry RED: the focused unit run failed with
  `TypeError: drawableViewerBox is not a function`; after the geometry
  helpers and padding-free measurement were added, the focused run passed 1
  file and 2 tests.
- Pointer routing RED: the focused browser test expected the touch-only route
  to reach Coast but mouse input reached Photo 0; retaining `pointerType`
  made the focused pointer suite pass 31 tests.
- Return-focus RED: focused browser coverage returned focus to Photo 0 and
  focused a tile after it disappeared; the current-asset target and wall
  fallback made both tests pass.
- Focus stability RED: a rerender returned focus to Back instead of Previous;
  split effects and filmstrip focus restoration made the focused test pass.
- Preview failure RED: a shared near-request rejection overwrote the current
  failure state; URL/generation-keyed request state made immediate failure and
  bounded retry tests pass.
- Normal-motion RED/GREEN: the new WebKit no-preference test was added with
  the crossfade assertions and passes in its dedicated project.

## Complete verification results

| Command | Result |
|---|---|
| `npm test` | PASS — 11 files, 91 tests |
| `npm run check` | PASS — Biome checked 57 files |
| `npm run typecheck` | PASS — `tsc -b --pretty false` |
| `npm run --workspace @photo-viewer/interface build` | PASS — Vite 1882 modules; JS 293.50 kB (gzip 90.27 kB), CSS 17.81 kB (gzip 4.32 kB) |
| `npm run test:browser` | PASS — 3 files, 90 tests |
| `npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion` | PASS — 1 file, 1 test |
| `cargo test --workspace --all-features` | PASS — 191 tests, 0 failed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | PASS — 11 tests, 0 failed |
| `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings` | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | PASS |
| `cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-viewer-final.json` | PASS — 10,000 assets; 6,602,752-byte database; 100-row first page; insert 820.510958 ms; first page 0.290583 ms; unavailable count 1.115167 ms; eviction plan 0.068791 ms |
| `npm run desktop:build -- --bundles app` | PASS — one unsigned macOS app bundle |
| `git diff --check` | PASS |

The browser suite emitted one expected React `act(...)` diagnostic from the
intentional invalid-image error-isolation harness; it did not produce an
unhandled test error and all 90 tests passed.

## Normal-motion evidence

The dedicated `browser-motion` project runs WebKit with
`prefers-reduced-motion: no-preference`. The test
“crossfades a decoded preview without changing fitted bounds” observed the
thumbnail during decode, a non-zero preview opacity transition, identical
fitted bounds before and after decode, and the ready preview. The regular
browser project continues to assert zero transition duration under reduced
motion.

## Full-range source audit

The exact audit commands were:

```text
git diff --name-only 0ccfa7c..HEAD
git diff --unified=0 0ccfa7c..HEAD -- apps/interface/src crates apps/desktop/src-tauri/src | rg -n -i '(writeFile|write_file|rename|removeFile|remove_file|unlink|copyFile|copy_file|setRating|updateRating|deleteAsset|sourcePath|selectedFolder|folderPath|locateFolder)' || true
```

The first command covered the complete production range, including all
approved Tasks 1–8 and the final wave. The second command returned only
path-free fixture/capability strings (`selectedFolderName` test fixtures and
`locateFolder: false`); it found no media write, rename, remove,
copy, rating mutation, tag mutation, asset deletion, or selected-folder path
operation. The viewer changes use derivative references and existing wall
interaction APIs only.

## Source fixture hash and controlled generation

The controlled source directory was `apps/interface/public/demo-photos`; the
user-selected folder was never named or accessed. The exact aggregate command
run immediately before and after actual derivative generation was:

```text
find apps/interface/public/demo-photos -maxdepth 1 -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256
```

Before: `a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6`

Actual controlled generation:

```text
cargo test -p photo-cache --test image_derivative controlled_demo_fixture_generation_leaves_source_unchanged
```

Result: PASS — 1 passed, 8 filtered out. The test generated the derivative in
a temporary cache and asserted source bytes and modification time remained
unchanged.

After: `a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6`

The six source-file SHA-256 values were:

```text
44faf249868e8d3ae81c0b532c460711526fbf17a66b82ee43ac805b3591cad1  city.jpg
ad6ba90e75709487ab6b927dc3f2781042e31312a4b7bc4606cddd4c0c5c4394  coast.jpg
91c69c673ef96b8afff1c36da486de4dece98fcaeb178310d00b675f2e48a699  forest.jpg
c8cc481a50bc60cdfb34e7e6ff2a0ac2e7350dd8f934afae91f3c19b0a2d3a05  interior.jpg
79bcb7af04b68935b29b5e0a80afb17d1823dde94a5fdbc75e3d5adf7a6e62ae  mountain.jpg
d1844a9747c7acfb36b10651ce79acff5c9d35b30a7b54323dbb171aed0bdfd0  portrait.jpg
```

## Allowed writes and artifacts

The allowed write locations remain SQLite application state, the managed
derivative cache, test temporary directories, Rust `target` outputs, Vite
`dist`, and the generated unsigned app bundle. The checked-in source fixture
is read-only for this workflow. No source write, rename, move, delete, rating,
tag, or user-selected-path operation was added.

The bundle is:

`apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`

After the final browser run, the generated directories
`apps/interface/.vitest-attachments` and
`apps/interface/src/components/__screenshots__` were removed. `git status
--short` is clean after the documentation follow-up commit, and
`git diff --check` passes.

## Native/running-app limitations

The unsigned macOS bundle was built successfully, and desktop Rust tests and
lint passed. I did not independently observe native GUI clicks, swipes,
rotation, source disconnection, or return-to-wall focus. The coordinator-owned
`wall-demo` app was already running and was left alive; it was not stopped,
restarted, or replaced. The automated browser tests use controlled in-memory
assets and cannot substitute for those native acceptance observations.

## Self-review and concerns

The final implementation stays within the approved viewer slice: no video
playback, locate-folder API, zoom/pan, signing, merge, push, or unrelated
refactor was added. Failure state and retry completion are fenced by asset,
generation, and URL. The only verification caveat is the expected React act
diagnostic in the intentional invalid-image browser harness; it is not a test
failure. Native GUI acceptance and the named desktop locate/reconnect follow-up
remain outside this slice.
