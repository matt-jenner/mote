# Thumbnail-first loading remediation implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the native photo wall reliably load viewport thumbnails first, retain them across sorting, show honest loading progress, and defer large previews until contact-sheet work is complete.

**Architecture:** The host-neutral interface derives deterministic visible, near, and remaining-page thumbnail passes from justified rows, while its controller owns a retryable per-epoch request lifecycle and atomic sort replacement. The Rust application service debounces screen-preview prefetch behind scan completion, an empty wall-thumbnail queue, and idle interaction. The standalone Tauri development profile optimises dependency code so image decoding and encoding represent production performance without removing desktop debugging.

**Tech Stack:** Rust 1.97.1 and edition 2024; SQLite through `photo-catalog`; Tokio 1.53.1; `image` 0.25.10; Tauri 2.11.5; React 19.2.8; TypeScript 7.0.2; Vitest 4.1.11 with Playwright WebKit; CSS Modules; Biome 2.5.10.

**Spec:** `docs/superpowers/specs/2026-08-26-thumbnail-first-loading-design.md`

## Global constraints

- Every production change follows strict red-green-refactor. Record the failing command and expected behavioral failure before implementation.
- Source media is read-only. Production code may read source files but may never write, rename, move, or delete them.
- SQLite stores metadata and derivative references, never image blobs. The managed cache remains the only writable image location.
- Wall thumbnails remain 1024-pixel durable derivatives. Screen previews remain 4096-pixel non-durable derivatives capped at source dimensions.
- The interface remains host-neutral. Only `apps/interface/src/services/tauriPhotoService.ts` may import Tauri APIs.
- Existing multiple-root identity, folder-group scoping, offline catalog behavior, cache-group eviction, warning convergence, and metadata precedence remain intact.
- New motion uses the existing fade token and becomes static under `prefers-reduced-motion`.
- Each task ends with focused verification and a logical commit. Do not merge or push.

---

### Task 1: Make contact-sheet loading deterministic, retryable, and observable

**Files:**
- Modify: `apps/interface/src/app/usePhotoWall.ts`
- Modify: `apps/interface/src/components/JustifiedWall.tsx`
- Modify: `apps/interface/src/components/PhotoWallCanvas.tsx`
- Modify: `apps/interface/src/components/WallToolbar.tsx`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/wall/wallReducer.ts`
- Modify: `apps/interface/src/styles/photoWall.module.css`
- Test: `apps/interface/src/wall/wallReducer.test.ts`
- Test: `apps/interface/src/components/PhotoWall.browser.test.tsx`

**Interfaces:**
- Consumes: `JustifiedRow[]`, `WallUpdate.progress`, `WallUpdate.derivativesReady`, `WallUpdate.warning`, `WallAsset.wallThumbnail`, and the existing visible/near `DerivativePriority` values.
- Produces: retryable per-epoch wall-thumbnail request state, deterministic row-pass requests, atomic direction replacement, and a host-neutral `WallProgress` view model consumed by `WallToolbar`.

- [ ] **Step 1: Add failing reducer tests for progress and sort retention**

Add focused tests that construct literal assets and verify these observable behaviors:

```ts
it("keeps populated thumbnails visible until the replacement sort page arrives", () => {
  const populated = loadedState([
    asset("old", { wallThumbnail: thumbnail("old") }),
    asset("new", { wallThumbnail: thumbnail("new") }),
  ]);
  const pending = wallReducer(populated, {
    type: "setDirection",
    direction: "newestFirst",
  });
  expect(pending.items.map((item) => item.id)).toEqual(["old", "new"]);
  expect(pending.items.every((item) => item.wallThumbnail !== null)).toBe(true);
  expect(pending.sortPending).toBe(true);
});

it("stores the latest scan counters for the active selection and generation", () => {
  const next = wallReducer(activeState(), {
    type: "progress",
    selectionId: "selection-a",
    generation: 3,
    progress: { discovered: 20, shaped: 12, enriched: 4, total: 20 },
  });
  expect(next.scanProgress).toEqual({
    discovered: 20,
    shaped: 12,
    enriched: 4,
    total: 20,
  });
});
```

The sort test catches clearing `items` in `setDirection`. The progress test catches ignored native progress updates. Use existing test fixture helpers where they already express complete `WallAsset` records.

- [ ] **Step 2: Run reducer tests and verify RED**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/wall/wallReducer.test.ts`

Expected: FAIL because direction changes clear items and no progress action/state exists.

- [ ] **Step 3: Add failing WebKit tests for deterministic passes and retry**

Add browser tests with `TestIntersectionObserver.fireOnObserve = false`:

```ts
it("requests the viewport, next two rows, then the remaining page without observer callbacks", async () => {
  // Render enough literal 4:3 assets for at least six rows.
  // Release the first wall page and never trigger TestIntersectionObserver.
  // Assert the first derivative request contains only viewport-row IDs with visible priority.
  // Assert the second request contains only the next two row IDs with nearViewport priority.
  // Flush the controlled idle callback and assert remaining IDs arrive in batches of at most 50.
  // Assert no asset ID appears in more than one pass unless promoted by a later scroll.
});

it("retries a still-missing thumbnail after changing sort direction", async () => {
  // Release an initial page without derivatives and observe its visible request.
  // Change to newest-first, then release the replacement page with the same asset still missing.
  // Assert a second visible request contains that asset ID.
});

it("keeps loaded images painted while the replacement sort query is pending", async () => {
  // Release a page whose assets have wallThumbnail references.
  // Change direction but hold the replacement query.
  // Assert the same image elements remain visible and the wall is aria-busy.
  // Release the replacement and assert the order swaps atomically and scrollTop is zero.
});

it("shows determinate contact-preview progress and quiet background large-preview progress", async () => {
  // Emit scan progress, release four assets, then release wall and screen derivative updates.
  // Assert accessible progress values and the exact stage transitions described by the spec.
});
```

Control idle work with a browser-test `requestIdleCallback` double when available and a timeout fallback otherwise. Assert rendered state and service requests, not the existence of the double.

- [ ] **Step 4: Run browser tests and verify RED**

Run: `npm run test:browser --workspace @photo-viewer/interface -- PhotoWall.browser.test.tsx`

Expected: FAIL because initial requests require observer callbacks, sort clears the wall, permanent request dedupe prevents retry, and no progress track exists.

- [ ] **Step 5: Implement retryable request lifecycle and atomic sorting**

In `wallReducer.ts`:

- add `scanProgress: ScanProgressDto | null` and `sortPending: boolean`;
- accept active-selection `progress` actions and the progress attached to `catalogBatch`;
- preserve `items` in `setDirection`, clear cursor/request state, increment `scrollEpoch`, and set `sortPending`;
- on the first matching `pageLoaded` after a sort, replace the visible page atomically instead of appending, merge existing `wallThumbnail` and `screenPreview` references by asset ID when the new row lacks them, then clear `sortPending`;
- keep stale source-generation, request-ID, cursor, epoch, warning-tombstone, and settlement fences unchanged.

In `usePhotoWall.ts`, replace the permanent `Map<string, DerivativePriority>` with per-source, per-sort-epoch request records. A request may be promoted from near to visible. Remove a record when a wall-thumbnail ready event arrives, when a retryable asset warning arrives, and when a new sort epoch starts for assets that remain without a thumbnail. Promise rejection must also make the batch eligible for retry. Do not retry screen-preview notifications through the wall-thumbnail request path.

- [ ] **Step 6: Implement three contact-sheet passes**

In `JustifiedWall.tsx`, derive row IDs from `rows` and the scroll container geometry:

- immediately request every missing thumbnail in rows whose cumulative vertical bounds intersect `scrollTop..scrollTop + clientHeight` as one `visible` batch;
- immediately request missing thumbnails in the next two rows as one `nearViewport` batch;
- enqueue remaining missing IDs from the current page in stable row order through `requestIdleCallback`, falling back to a zero-delay timer, in batches no larger than 50;
- retain observers so scrolling promotes newly visible IDs and the load sentinel fetches more catalog pages;
- filter assets already holding `wallThumbnail` before every pass;
- cancel idle callbacks/timers when rows, source, sort epoch, or component lifetime changes.

Pass `scrollEpoch` into `JustifiedWall` so a sort creates a fresh loading pass even when IDs repeat. Keep the existing RAF batching for observer callbacks.

- [ ] **Step 7: Implement the accessible toolbar progress model**

Create a small `WallProgress` discriminated union or equivalent derived view model in `usePhotoWall.ts`. Count known wall and screen references directly from `state.items`.

The status priority is:

1. current error or warning;
2. contact thumbnails missing: `Preparing previews · X of Y` with `value=X`, `max=Y`;
3. contact thumbnails ready and screen previews missing after scan completion: `Photos ready · preparing larger previews · X of Y`;
4. no shaped items yet: indexing text and `shaped/total` when total is known;
5. complete ready or empty text.

Render a native `<progress>` element or an equivalent correctly labelled progressbar in `WallToolbar.tsx`. Keep the track thin and subordinate to sort controls. Apply `aria-busy` to the wall during an active query or missing contact thumbnails. Add indeterminate styling without animated movement under reduced motion.

- [ ] **Step 8: Verify Task 1**

Run:

```bash
npm test
npm run test:browser
npm run typecheck
npm run check
```

Expected: all interface unit and WebKit tests pass with no console errors, accessibility violations, formatting errors, or type errors.

- [ ] **Step 9: Commit Task 1**

```bash
git add apps/interface docs/superpowers/specs/2026-08-26-thumbnail-first-loading-design.md docs/superpowers/plans/2026-08-26-thumbnail-first-loading-remediation.md
git commit -m "fix: load contact sheet thumbnails in priority passes"
```

---

### Task 2: Defer large previews and optimise native development image work

**Files:**
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/service.rs` only if shared preview-gate state is needed
- Modify: `crates/app-service/tests/progressive_wall.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `docs/superpowers/verification/2026-08-25-macos-progressive-photo-wall.md`

**Interfaces:**
- Consumes: `DerivativeQueue::is_empty`, `IndexScheduler::available_background_permits`, active-scan state, interaction mode, active `SelectionToken`, and recently requested asset IDs.
- Produces: a debounced preview-prefetch gate that starts only after contact-sheet and scan work drain, plus optimised dependency code in `tauri dev`.

- [ ] **Step 1: Add failing overlapping-wave preview test**

Add an integration test using a blocking derivative boundary already available in the fixture or a deterministic test gate:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screen_previews_wait_for_all_overlapping_wall_thumbnail_waves() {
    // Start a settled four-image fixture.
    // Issue visible IDs [a, b] and near IDs [c, d] as overlapping requests.
    // Hold the second wall wave while the first finishes.
    // Assert no ScreenPreview update arrives while any WallThumbnail work remains.
    // Release the second wave, return interaction to idle, and assert all four wall
    // references arrive before the first ScreenPreview reference.
}
```

The test catches spawning `prefetch_screen_previews(ids)` at the end of each individual request.

- [ ] **Step 2: Run the focused Rust test and verify RED**

Run: `cargo test -p photo-app-service --test progressive_wall screen_previews_wait_for_all_overlapping_wall_thumbnail_waves -- --exact`

Expected: FAIL because the first request can start screen-preview work before the later contact-sheet wave drains.

- [ ] **Step 3: Implement the idle preview gate**

Replace immediate per-request preview spawning with one selection-fenced, debounced gate:

- accumulate recently requested IDs in stable de-duplicated order;
- after a wall request finishes, schedule one short quiet-period check rather than starting a preview directly;
- abort or reschedule if an active scan exists, the derivative queue is non-empty, the scheduler exposes only active-interaction capacity, or the selection token changed;
- when all gates pass, generate recent screen previews first and then the remaining group with `IdleLibrary` priority;
- if interaction becomes idle and work was deferred, wake the same gate;
- a new visible wall request invalidates a pending preview start and stays higher priority than any queued preview;
- keep selection checks before cache-budget, write, and catalog registration side effects.

Do not introduce an unbounded task per request. Store one generation/token or join handle in service state so stale debounce tasks exit harmlessly.

- [ ] **Step 4: Optimise Tauri development dependencies**

Add this standalone profile to `apps/desktop/src-tauri/Cargo.toml`:

```toml
[profile.dev.package."*"]
opt-level = 3
```

This keeps `photo-viewer-desktop` debuggable while optimising dependency and path-dependency image work. Do not change release settings or the root Cargo workspace profile.

- [ ] **Step 5: Measure the controlled derivative workload**

Record fresh timings for the same representative derivative test workload before and after the standalone development profile takes effect. Also record root debug and release timings as context:

```bash
/usr/bin/time -p cargo test -p photo-cache --test image_derivative
/usr/bin/time -p cargo test --release -p photo-cache --test image_derivative
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
```

If the desktop manifest cannot directly select the cache integration test because it is a path dependency, use a small ignored benchmark harness or the existing app-service fixture through the desktop manifest. Do not add a source-text assertion for the profile. Report compile-warm execution timings separately from compilation.

- [ ] **Step 6: Verify Task 2**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check
```

Expected: all tests, Clippy checks, and formatting checks pass. Source fixture hashes remain unchanged.

- [ ] **Step 7: Commit Task 2**

```bash
git add crates/app-service apps/desktop/src-tauri/Cargo.toml docs/superpowers/verification/2026-08-25-macos-progressive-photo-wall.md
git commit -m "fix: defer large previews behind thumbnail work"
```

---

### Task 3: Verify and relaunch the native exploratory slice

**Files:**
- Modify: `docs/superpowers/verification/2026-08-25-macos-progressive-photo-wall.md`

**Interfaces:**
- Consumes: Task 1 and Task 2 commits.
- Produces: fresh verification evidence and a running development app for user exploration.

- [ ] **Step 1: Run the full repository verification set**

Run interface unit, WebKit, typecheck, Biome, interface build, full Rust workspace tests, full root and desktop Clippy, both format checks, benchmark smoke, and unsigned macOS bundle creation. Run `git diff --check` and confirm the worktree is clean after the verification commit.

- [ ] **Step 2: Audit the source-media invariant**

Inspect the production diff for source-path write, rename, move, copy, or delete operations. Hash controlled source fixtures before and after derivative generation and record equality. Do not expose the user's selected native path.

- [ ] **Step 3: Run a clean-profile native smoke**

Launch the app with a fresh named profile, select a controlled photo folder, and record:

- time from geometry paint to first viewport thumbnail;
- time until the current viewport is fully refined;
- time until all contact-sheet thumbnails for the page are ready;
- whether larger previews begin only after contact thumbnails and idle state;
- sort behavior while a replacement query is pending;
- progress stage transitions.

If macOS blocks UI automation, keep the app running for user exploration and state that the native interaction remains user-verified.

- [ ] **Step 4: Update verification evidence and commit**

Update the verification document with exact commands, counts, timing evidence, native limitations, and the new commits. Commit only tracked verification artifacts:

```bash
git add docs/superpowers/verification/2026-08-25-macos-progressive-photo-wall.md
git commit -m "test: verify thumbnail-first native loading"
```

Do not merge or push. Leave the app running for the user's exploratory check.
