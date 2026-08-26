# Immersive filmstrip photo viewer implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open any available wall photo in a fast edge-to-edge viewer with progressive screen-preview refinement, bounded ordered navigation, a filmstrip, basic information, touch swipes, and rotation-safe mobile layout.

**Architecture:** Keep the wall mounted under a host-neutral React overlay. A small sequence interface reads the wall controller's current order and pagination, while focused viewer hooks own navigation state, derivative refinement, controls, gestures, and viewport measurement. Extend the existing Rust and TypeScript service contract with resolved rating and an explicit derivative class so the viewer can request 4096-pixel previews directly without weakening the source-read-only boundary.

**Tech Stack:** Rust 1.97.1 and edition 2024; SQLite through `photo-catalog`; Tokio 1.53.1; Tauri 2.11.5; React 19.2.8; TypeScript 7.0.2; Vitest 4.1.11 with Playwright WebKit; CSS Modules; Lucide React 1.34.0; Biome 2.5.10.

**Spec:** `docs/superpowers/specs/2026-08-26-immersive-filmstrip-viewer-design.md`

## Global constraints

- Every production change follows strict red-green-refactor. Run each named focused test before and after its implementation.
- Use a fresh `gpt-5.6-luna` implementation subagent for each task and complete specification and code-quality review before accepting that task.
- Source media is read-only. Production code may read source files but may never write, rename, move, rate, tag, or delete them.
- SQLite stores metadata and derivative references, never image blobs. The managed cache remains the only writable image location.
- The interface remains host-neutral. Only `apps/interface/src/services/tauriPhotoService.ts` may import Tauri APIs.
- Wall thumbnails remain 1024-pixel derivatives. Screen previews remain 4096-pixel derivatives capped at source dimensions.
- The first viewer frame never waits for a screen preview when a wall thumbnail or stable placeholder is available.
- Viewer navigation follows the wall's current loaded order and never loads the complete result set.
- The information drawer is read-only and non-modal. Ordinary image taps do not open it.
- Horizontal swipe navigates only in the fit-to-window state. The gesture interface must leave room for a later zoomed state where drag pans instead.
- Mobile rotation and desktop resize preserve the current asset, best decoded derivative, drawer state, controls state, and centred filmstrip selection.
- Motion becomes immediate under `prefers-reduced-motion`.
- Each task ends with focused verification and a logical commit. Do not merge or push.
- Demonstrate each visible checkpoint before beginning the next visible checkpoint.

## Planned file map

### Shared contract and backend

- `crates/catalog/src/wall_repo.rs` projects resolved rating with wall records.
- `crates/app-service/src/dto.rs` adds `WallAsset.rating` and `DerivativeRequest.kind`.
- `crates/app-service/src/service.rs` maps catalog rating into the path-free wall DTO.
- `crates/app-service/src/derivatives.rs` honours explicit wall-thumbnail and screen-preview requests.
- `apps/interface/src/services/photoService.ts` mirrors the shared DTO contract.
- `apps/interface/src/services/inMemoryPhotoService.ts` supports explicit derivative-class requests in browser fixtures.
- `apps/interface/src/services/tauriPhotoService.ts` keeps the transport mapping unchanged apart from the typed request payload.

### Viewer state and presentation

- `apps/interface/src/viewer/viewerReducer.ts` owns viewer lifecycle, current asset, drawer, controls, and request generation.
- `apps/interface/src/viewer/photoSequence.ts` calculates ordered positions, neighbour windows, and pagination thresholds.
- `apps/interface/src/viewer/useViewerPreview.ts` requests and resolves current and neighbouring derivatives without stale replacement.
- `apps/interface/src/viewer/useViewerControls.ts` owns pointer and touch visibility timers.
- `apps/interface/src/viewer/useViewerGestures.ts` classifies taps, swipes, drawer scrolls, and the future zoomed policy.
- `apps/interface/src/viewer/useViewerViewport.ts` coalesces visual viewport, resize, and orientation changes.
- `apps/interface/src/components/PhotoViewerOverlay.tsx` composes the viewer and controls overlay.
- `apps/interface/src/components/ViewerStage.tsx` paints fixed-geometry progressive derivative layers.
- `apps/interface/src/components/ViewerFilmstrip.tsx` renders a bounded thumbnail window and centres selection.
- `apps/interface/src/components/PhotoInfoDrawer.tsx` renders the approved catalog-backed fields.
- `apps/interface/src/styles/photoViewer.module.css` contains viewer-only responsive, safe-area, and reduced-motion styles.

### Wall integration and verification

- `apps/interface/src/components/AppShell.tsx` keeps overlay lifecycle above the mounted wall.
- `apps/interface/src/components/PhotoWallCanvas.tsx` exposes the wall scroll element to the overlay owner.
- `apps/interface/src/components/JustifiedWall.tsx` passes open and highlight actions to tiles.
- `apps/interface/src/components/PhotoTile.tsx` becomes an accessible open control for viewable assets.
- `apps/interface/src/components/PhotoViewer.browser.test.tsx` exercises the visible viewer flow in WebKit.
- `docs/superpowers/verification/2026-08-26-immersive-filmstrip-viewer.md` records commands, native evidence, and the read-only audit.

---

### Task 1: Extend the path-free wall and derivative contract

**Files:**
- Modify: `crates/catalog/src/wall_repo.rs`
- Modify: `crates/catalog/tests/wall_query.rs`
- Modify: `crates/app-service/src/dto.rs`
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/tests/progressive_wall.rs`
- Modify: `apps/desktop/src-tauri/src/protocol.rs`
- Modify: `apps/interface/src/services/photoService.ts`
- Modify: `apps/interface/src/services/photoService.test.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.test.ts`
- Modify: `apps/interface/src/services/inMemoryPhotoService.ts`
- Modify: `apps/interface/src/components/PhotoWall.browser.test.tsx`
- Modify: `apps/interface/src/wall/layoutJustifiedRows.test.ts`
- Modify: `apps/interface/src/wall/wallReducer.test.ts`
- Modify: all `WallAsset` and `DerivativeRequest` literals in `apps/desktop/src-tauri/src/protocol.rs` and `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**
- Consumes: catalog `assets.rating`, `DerivativeClass::{WallThumbnail, ScreenPreview}`, existing derivative cache and scheduler, and current `WallUpdate::DerivativesReady` events.
- Produces: `WallAsset.rating: Option<u8>` in Rust, `WallAsset.rating: number | null` in TypeScript, and `DerivativeRequest { asset_ids, priority, kind }` or its camel-case TypeScript equivalent.

- [ ] **Step 1: Add failing catalog and DTO tests for rating**

Extend a wall-query fixture with rating `4` and assert the projected record and serialized DTO retain it:

```rust
assert_eq!(page.items[0].rating, Some(4));

let value = serde_json::to_value(WallAsset {
    id: "00000000-0000-0000-0000-000000000001".to_owned(),
    display_name: "photo.jpg".to_owned(),
    media_kind: WallMediaKind::Jpeg,
    provisional_order: 1,
    captured_at_utc: Some("2026-08-26T12:00:00Z".to_owned()),
    date_state: OrderState::Settled,
    width: 2048,
    height: 1365,
    representative_rgb: Some(0x334455),
    shape_state: WallShapeState::Ready,
    availability: SourceAvailability::Available,
    warning: None,
    wall_thumbnail: None,
    screen_preview: None,
    rating: Some(4),
}).unwrap();
assert_eq!(value["rating"], 4);
assert!(!value.to_string().contains("/photos/"));
```

Add `rating: None` to unaffected literal fixtures so the compiler identifies every contract consumer.

- [ ] **Step 2: Run the focused rating tests and verify RED**

Run:

```bash
cargo test -p photo-catalog --test wall_query
cargo test -p photo-app-service dto::tests
```

Expected: compilation or assertions fail because `WallCatalogRecord` and `WallAsset` do not expose rating.

- [ ] **Step 3: Project rating through catalog and application DTOs**

Add the field to both records:

```rust
pub struct WallCatalogRecord {
    // existing fields
    pub rating: Option<u8>,
}

pub struct WallAsset {
    // existing fields
    pub rating: Option<u8>,
}
```

Select `rating` immediately after `shape_status` in both `wall_records_for_assets` and `wall_page`, then decode it with checked conversion:

```rust
rating: row
    .get::<_, Option<i64>>(10)?
    .map(|value| u8::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            10,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    }))
    .transpose()?,
```

Shift warning columns to 11 and 12. Set `rating: item.rating` in `wall_asset_from_record`.

- [ ] **Step 4: Add failing explicit screen-preview request tests**

Add a Rust integration test which requests one known asset directly:

```rust
service
    .request_derivatives(DerivativeRequest {
        asset_ids: vec![asset_id.clone()],
        priority: DerivativePriority::Visible,
        kind: DerivativeClass::ScreenPreview,
    })
    .await
    .unwrap();

let ready = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 1).await;
assert_eq!(ready[0].asset_id, asset_id);
```

Add TypeScript contract assertions:

```ts
const request: DerivativeRequest = {
  assetIds: ["asset-a"],
  priority: "visible",
  kind: "screenPreview",
};
await service.requestDerivatives(request);
expect(invoke).toHaveBeenCalledWith("request_derivatives", { request });
```

- [ ] **Step 5: Run focused derivative contract tests and verify RED**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall explicit_visible_screen_preview_request -- --exact
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/photoService.test.ts src/services/tauriPhotoService.test.ts
```

Expected: compilation or assertions fail because requests have no derivative kind and the service always resolves wall thumbnails.

- [ ] **Step 6: Implement explicit derivative-class routing**

Change the shared request:

```rust
pub struct DerivativeRequest {
    pub asset_ids: Vec<String>,
    pub priority: DerivativePriority,
    pub kind: DerivativeClass,
}

impl DerivativeRequest {
    pub fn visible(asset_ids: Vec<String>) -> Self {
        Self {
            asset_ids,
            priority: DerivativePriority::Visible,
            kind: DerivativeClass::WallThumbnail,
        }
    }

    pub fn visible_screen_preview(asset_ids: Vec<String>) -> Self {
        Self {
            asset_ids,
            priority: DerivativePriority::Visible,
            kind: DerivativeClass::ScreenPreview,
        }
    }
}
```

In `request_derivatives`, pass `request.kind` to `resolve_derivatives`. Keep wall-only prefetch bookkeeping behind `request.kind == DerivativeClass::WallThumbnail`. A direct screen-preview request must run through the same bounded queue at the requested visible or near-viewport priority and publish its ready reference through the existing ordered update channel. Preserve the 250-ID limit and active-selection checks.

Mirror the request in TypeScript:

```ts
export interface DerivativeRequest {
  assetIds: string[];
  priority: DerivativePriority;
  kind: DerivativeClass;
}
```

Update every wall caller to send `kind: "wallThumbnail"`. The in-memory service must record the complete request and publish only the requested fixture derivative class when a test asks it to simulate completion.

- [ ] **Step 7: Verify Task 1**

Run:

```bash
cargo test -p photo-catalog --test wall_query
cargo test -p photo-app-service --test progressive_wall
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm test
npm run typecheck
npm run check
```

Expected: all focused Rust, desktop, TypeScript, and formatting checks pass. Serialized errors and DTOs remain path-free.

- [ ] **Step 8: Commit Task 1**

```bash
git add crates/catalog crates/app-service apps/desktop/src-tauri apps/interface/src
git commit -m "feat: request viewer previews through shared contract"
```

---

### Task 2: Add deterministic viewer state and sequence boundaries

**Files:**
- Create: `apps/interface/src/viewer/viewerReducer.ts`
- Create: `apps/interface/src/viewer/viewerReducer.test.ts`
- Create: `apps/interface/src/viewer/photoSequence.ts`
- Create: `apps/interface/src/viewer/photoSequence.test.ts`

**Interfaces:**
- Consumes: ordered `readonly WallAsset[]`, stable asset IDs, `nextCursor`, active query state, and captured wall scroll position.
- Produces: `ViewerState`, `viewerReducer`, `ViewerReturnAnchor`, `findViewerIndex`, `viewerNeighbourIds`, `viewerFilmstripWindow`, and `shouldLoadViewerPage`.

- [ ] **Step 1: Write failing viewer reducer tests**

Use exact state transitions:

```ts
it("opens at the selected asset and increments the preview generation on navigation", () => {
  const opened = viewerReducer(initialViewerState, {
    type: "open",
    assetId: "b",
    anchor: { assetId: "b", scrollTop: 640 },
  });
  const moved = viewerReducer(opened, { type: "select", assetId: "c" });
  expect(moved.currentAssetId).toBe("c");
  expect(moved.previewGeneration).toBe(opened.previewGeneration + 1);
});

it("keeps information open across navigation and resets it after close", () => {
  const open = viewerReducer(initialViewerState, {
    type: "open",
    assetId: "a",
    anchor: { assetId: "a", scrollTop: 20 },
  });
  const info = viewerReducer(open, { type: "setInfoOpen", open: true });
  expect(viewerReducer(info, { type: "select", assetId: "b" }).infoOpen).toBe(true);
  expect(viewerReducer(info, { type: "close" }).infoOpen).toBe(false);
});
```

- [ ] **Step 2: Write failing sequence tests**

```ts
expect(findViewerIndex(items, "b")).toBe(1);
expect(viewerNeighbourIds(items, 1, 2)).toEqual({
  immediate: ["a", "c"],
  idle: ["d"],
});
expect(viewerFilmstripWindow(items, 50, 15)).toEqual({ start: 35, end: 66 });
expect(shouldLoadViewerPage(96, 100, true, false)).toBe(true);
expect(shouldLoadViewerPage(99, 100, true, true)).toBe(false);
```

- [ ] **Step 3: Run unit tests and verify RED**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerReducer.test.ts src/viewer/photoSequence.test.ts
```

Expected: FAIL because the viewer modules do not exist.

- [ ] **Step 4: Implement the reducer and sequence helpers**

Use these public shapes:

```ts
export interface ViewerReturnAnchor {
  assetId: string;
  scrollTop: number;
}

export interface ViewerState {
  open: boolean;
  currentAssetId: string | null;
  returnAnchor: ViewerReturnAnchor | null;
  infoOpen: boolean;
  controlsVisible: boolean;
  filmstripVisible: boolean;
  previewGeneration: number;
}
```

The reducer handles `open`, `close`, `select`, `setInfoOpen`, `showControls`, `hideControls`, and `toggleTouchControls`. `close` resets all transient viewer state. `select` changes only current asset and preview generation. The pure sequence helpers preserve input order, use a five-item pagination threshold, and return a bounded filmstrip range of at most 31 assets.

- [ ] **Step 5: Verify and commit Task 2**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerReducer.test.ts src/viewer/photoSequence.test.ts
npm run typecheck
npm run check
```

Then commit:

```bash
git add apps/interface/src/viewer
git commit -m "feat: define viewer navigation state"
```

---

### Task 3: Deliver the open, view, and exact-return checkpoint

**Files:**
- Create: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Create: `apps/interface/src/components/ViewerStage.tsx`
- Create: `apps/interface/src/styles/photoViewer.module.css`
- Create: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/components/PhotoWallCanvas.tsx`
- Modify: `apps/interface/src/components/JustifiedWall.tsx`
- Modify: `apps/interface/src/components/PhotoTile.tsx`
- Modify: `apps/interface/src/styles/photoWall.module.css`

**Interfaces:**
- Consumes: Task 2 `ViewerState`, `viewerReducer`, ordered wall assets, `PhotoService.derivativeUrl`, and the wall region element.
- Produces: accessible tile opening, an edge-to-edge overlay with immediate best-ready derivative, and exact scroll and focus restoration on close.

- [ ] **Step 1: Add failing WebKit tests for open and return**

Add controlled wall assets with ready thumbnails:

```tsx
it("opens the selected tile above the mounted wall and returns to its exact position", async () => {
  const view = renderViewerWall(serviceWithReadyPhotos());
  const wall = view.getByRole("region", { name: "Photos" });
  wall.scrollTop = 420;
  await view.getByRole("button", { name: "Open Coast" }).click();
  await expect.element(view.getByRole("dialog", { name: "Photo viewer" })).toBeVisible();
  expect(document.querySelector("[data-testid='photo-wall']")).not.toBeNull();
  await view.getByRole("button", { name: "Back to photos" }).click();
  expect(wall.scrollTop).toBe(420);
  await expect.element(view.getByRole("button", { name: "Open Coast" })).toHaveFocus();
});

it("uses the wall thumbnail for the first frame", async () => {
  await openAsset("Coast");
  const image = document.querySelector<HTMLImageElement>("[data-viewer-layer='wallThumbnail']");
  expect(image?.src).toContain("coast-wall");
});
```

- [ ] **Step 2: Run the browser test and verify RED**

Run: `npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx`

Expected: FAIL because tiles do not open a viewer and no overlay exists.

- [ ] **Step 3: Make tiles accessible open controls**

Change the tile root to a button when it is viewable:

```tsx
<button
  aria-label={`Open ${asset.displayName}`}
  className={styles.tile}
  data-asset-id={asset.id}
  onClick={() => onOpen(asset.id)}
  ref={tileRef}
  style={style}
  type="button"
>
  {layers}
</button>
```

Use an inert figure for an unavailable asset with no derivative. Do not open a viewer or local folder picker from that state. Keep the existing question-mark treatment and fixed row geometry.

- [ ] **Step 4: Mount the overlay without unmounting the wall**

Lift the wall region ref to `AppShell`. On tile open, dispatch:

```ts
dispatchViewer({
  type: "open",
  assetId,
  anchor: { assetId, scrollTop: wallRegionRef.current?.scrollTop ?? 0 },
});
```

Render `PhotoViewerOverlay` after the workspace, with `position: fixed` and `inset: 0`. Set the covered workspace inert while the viewer is open. `ViewerStage` computes the fitted rectangle from catalog width and height and paints screen preview, wall thumbnail, or representative colour in that order. Closing restores `scrollTop` in a layout effect, focuses the originating tile, and applies a 600-millisecond non-moving highlight class.

The top-left back button uses a Lucide chevron, `aria-label="Back to photos"`, and a 44-pixel safe-area-aware target. Clicking the dark stage background does nothing.

- [ ] **Step 5: Verify and commit Task 3**

Run:

```bash
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
npm test
npm run typecheck
npm run check
```

Then commit:

```bash
git add apps/interface/src
git commit -m "feat: open photos in immersive viewer"
```

- [ ] **Step 6: Demonstrate checkpoint 1**

Start the macOS development app with an isolated profile. Open several wall tiles and verify immediate paint, wall preservation, back-chevron return, exact scroll restoration, and tile focus. Record the result in the verification document before beginning Task 4.

---

### Task 4: Refine the stage to current and neighbouring screen previews

**Files:**
- Create: `apps/interface/src/viewer/viewerPreviewPlan.ts`
- Create: `apps/interface/src/viewer/viewerPreviewPlan.test.ts`
- Create: `apps/interface/src/viewer/useViewerPreview.ts`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/ViewerStage.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`

**Interfaces:**
- Consumes: explicit Task 1 derivative requests, Task 2 neighbour helpers and preview generation, wall asset derivative updates, and `PhotoService.derivativeUrl`.
- Produces: `useViewerPreview({ assets, currentIndex, previewGeneration })` with current URL, base URL, failure state, and bounded priority requests.

- [ ] **Step 1: Add failing preview priority and stale-result tests**

```ts
it("requests the current preview visibly and immediate neighbours near the viewport", async () => {
  expect(buildViewerPreviewPlan(fiveAssets, 2)).toEqual([{
    assetIds: ["c"], priority: "visible", kind: "screenPreview",
  }, {
    assetIds: ["b", "d"], priority: "nearViewport", kind: "screenPreview",
  }, {
    assetIds: ["a", "e"], priority: "nearViewport", kind: "screenPreview",
    idle: true,
  }]);
});

it("does not replace the current photo with an older decode completion", async () => {
  const view = await openAsset("B");
  await navigateTo("C");
  releaseDecode("b-screen");
  expect(view.getByTestId("viewer-stage").getAttribute("data-current-asset")).toBe("c");
});
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerPreviewPlan.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
```

Expected: FAIL because the viewer does not request or fence screen-preview refinement.

- [ ] **Step 3: Implement bounded preview scheduling**

`buildViewerPreviewPlan` returns current, immediate-neighbour, and idle-neighbour request groups without reading browser state. The hook sends these exact request shapes:

```ts
void service.requestDerivatives({
  assetIds: [current.id],
  priority: "visible",
  kind: "screenPreview",
});

void service.requestDerivatives({
  assetIds: immediateNeighbourIds,
  priority: "nearViewport",
  kind: "screenPreview",
});
```

After browser idle, request the next two neighbours in each direction with near-viewport priority. De-duplicate per asset and derivative key, but allow retry after a rejected request or retryable warning. Cancel the idle callback on navigation and unmount.

- [ ] **Step 4: Implement fixed-stage decode and crossfade**

Keep the wall thumbnail mounted as the base layer. Create a screen-preview image for the current generation and call `decode()` before marking it ready. Every completion checks both asset ID and preview generation. Crossfade only opacity inside the fitted rectangle. On failure, retain the base layer and expose `largePreviewUnavailable: true` to the information UI without blanking the stage.

Under reduced motion, set the ready layer opacity immediately and remove transition duration. A resize must reuse the same decoded image element and only change fitted bounds.

- [ ] **Step 5: Verify and commit Task 4**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerPreviewPlan.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
npm run typecheck
npm run check
```

Then commit:

```bash
git add apps/interface/src
git commit -m "feat: refine viewer photos from preview cache"
```

---

### Task 5: Add bounded navigation and the centred filmstrip

**Files:**
- Create: `apps/interface/src/components/ViewerFilmstrip.tsx`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`
- Modify: `apps/interface/src/app/usePhotoWall.ts` only to expose existing query and derivative methods required by the sequence interface

**Interfaces:**
- Consumes: Task 2 sequence helpers, `wall.state.items`, `wall.state.cursor`, `wall.loading`, `wall.loadMore`, and `wall.requestNearViewportDerivatives`.
- Produces: equivalent arrow-key, previous/next-button, and filmstrip navigation with no wrapping and bounded filmstrip DOM.

- [ ] **Step 1: Add failing navigation and pagination tests**

```tsx
it("navigates through the wall order without wrapping", async () => {
  await openAsset("B");
  await user.keyboard("{ArrowRight}");
  await expect.element(view.getByRole("img", { name: "C" })).toBeVisible();
  await user.keyboard("{ArrowLeft}");
  await expect.element(view.getByRole("img", { name: "B" })).toBeVisible();
  await navigateToFirst();
  await expect.element(view.getByRole("button", { name: "Previous photo" })).toBeDisabled();
});

it("loads another wall page near the loaded edge and keeps next disabled while pending", async () => {
  await openAsset("Photo 96");
  expect(service.queryRequests).toHaveLength(2);
  await navigateToLoadedEnd();
  await expect.element(view.getByRole("button", { name: "Next photo" })).toBeDisabled();
  service.releaseQuery(1, nextPage);
  await expect.element(view.getByRole("button", { name: "Next photo" })).toBeEnabled();
});
```

Add a filmstrip assertion that no more than 31 thumbnail buttons mount for 100 loaded assets and that the current button has `aria-current="true"`.

- [ ] **Step 2: Run the browser tests and verify RED**

Run: `npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx`

Expected: FAIL because the overlay has no ordered navigation or filmstrip.

- [ ] **Step 3: Implement equivalent navigation paths**

Create one `selectAsset(assetId)` action and route every navigation path through it. Attach key handling to the overlay, ignoring keys originating in editable controls. Previous and next buttons are 44-pixel targets and disabled at unavailable boundaries. Do not wrap.

Call `loadMore()` when `shouldLoadViewerPage` becomes true. While the next page is pending at the loaded end, leave the current asset visible and next disabled. When wall items append, the same current asset ID resolves to its unchanged index and navigation continues.

- [ ] **Step 4: Implement the bounded filmstrip**

Render only `viewerFilmstripWindow(items, currentIndex, 15)`. Each item is a button with the photo name, ready wall thumbnail or representative colour, and `aria-current` for the selected asset. Request missing thumbnails for that window with near-viewport priority.

On selection or viewport revision, call the current item's `scrollIntoView({ inline: "center", block: "nearest" })` unless reduced motion is active, in which case use immediate scrolling. Keep the current thumbnail centred where neighbouring content permits.

- [ ] **Step 5: Verify and commit Task 5**

Run:

```bash
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
npm test
npm run typecheck
npm run check
```

Then commit:

```bash
git add apps/interface/src
git commit -m "feat: navigate photos with centred filmstrip"
```

- [ ] **Step 6: Demonstrate checkpoint 2**

Run the macOS app and demonstrate keyboard arrows, visible controls, direct filmstrip selection, no wrapping, and loading another catalog page near the current boundary. Record the result before Task 6.

---

### Task 6: Add information and quiet auto-hiding controls

**Files:**
- Create: `apps/interface/src/components/PhotoInfoDrawer.tsx`
- Create: `apps/interface/src/viewer/useViewerControls.ts`
- Create: `apps/interface/src/viewer/useViewerControls.test.ts`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`

**Interfaces:**
- Consumes: `WallAsset` catalog metadata, viewer drawer state, preview failure state, pointer capability media queries, and reduced-motion preference.
- Produces: explicit Info drawer, desktop and touch control timers, Escape precedence, and accessible current-photo announcements.

- [ ] **Step 1: Write failing timer and drawer tests**

```ts
it("uses the desktop and touch inactivity delays", () => {
  expect(controlHideDelay("mouse")).toBe(2500);
  expect(controlHideDelay("touch")).toBe(3500);
});
```

```tsx
it("opens information only from Info and keeps it open during navigation", async () => {
  await openAsset("Coast");
  await tapStage();
  expect(view.queryByRole("complementary", { name: "Photo information" })).toBeNull();
  await view.getByRole("button", { name: "Photo information" }).click();
  await expect.element(view.getByText("4 stars")).toBeVisible();
  await user.keyboard("{ArrowRight}");
  await expect.element(view.getByRole("complementary", { name: "Photo information" })).toBeVisible();
});

it("closes information on the first Escape and returns to the wall on the second", async () => {
  await openInfo();
  await user.keyboard("{Escape}");
  expect(infoDrawer()).toBeNull();
  await user.keyboard("{Escape}");
  expect(viewer()).toBeNull();
});
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/useViewerControls.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
```

Expected: FAIL because the viewer has no information drawer, timers, or Escape precedence.

- [ ] **Step 3: Implement the read-only overlay drawer**

Render filename, locally formatted capture date and time, rating as `Unrated`, `1 star`, or `N stars`, oriented dimensions as `W × H`, and the human-readable media type. Use catalog data only. Display `Larger preview unavailable` when the current preview request failed.

The drawer is a labelled non-modal `aside`. It overlays the right 320 pixels on desktop and 88 percent of usable width on a phone. It may scroll vertically and never traps focus. Keep it mounted and visible across photo navigation. Reset it only when the viewer closes.

- [ ] **Step 4: Implement auto-hiding controls and announcements**

`useViewerControls` exposes `showForInput("mouse" | "touch")`, `toggleTouch()`, `keepVisible()`, and the current visibility. Pointer movement restarts a 2500-millisecond timer. Touch toggle uses 3500 milliseconds. Focus inside controls pauses hiding. Drawer controls remain available while the drawer is open.

Entering the bottom 96-pixel reveal zone with a pointer shows the filmstrip. A touch tap on the stage toggles both primary controls and the filmstrip. Neither path opens the information drawer.

When controls hide, remove them from pointer and keyboard interaction rather than leaving invisible focus targets. Add an `aria-live="polite"` status containing the filename and position. Say `photo N of M loaded` while `nextCursor` is non-null.

- [ ] **Step 5: Verify and commit Task 6**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/useViewerControls.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
npm run typecheck
npm run check
```

Then commit:

```bash
git add apps/interface/src
git commit -m "feat: add viewer information and quiet controls"
```

- [ ] **Step 6: Demonstrate checkpoint 3**

Run the macOS app and demonstrate the explicit Info drawer, live metadata changes during navigation, controls fading and returning, bottom filmstrip reveal, and two-step Escape behaviour. Record the result before Task 7.

---

### Task 7: Add touch swipe and rotation-safe responsive layout

**Files:**
- Create: `apps/interface/src/viewer/useViewerGestures.ts`
- Create: `apps/interface/src/viewer/useViewerGestures.test.ts`
- Create: `apps/interface/src/viewer/viewerViewport.ts`
- Create: `apps/interface/src/viewer/viewerViewport.test.ts`
- Create: `apps/interface/src/viewer/useViewerViewport.ts`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/ViewerStage.tsx`
- Modify: `apps/interface/src/components/ViewerFilmstrip.tsx`
- Modify: `apps/interface/src/components/PhotoInfoDrawer.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`

**Interfaces:**
- Consumes: pointer events, `window.visualViewport`, safe-area CSS variables, Task 2 select action, current fit policy, current decoded preview layer, and filmstrip centring.
- Produces: `classifyViewerGesture`, `useViewerGestures`, and coalesced `{ width, height, revision }` viewport state.

- [ ] **Step 1: Add failing gesture classification tests**

```ts
expect(classifyViewerGesture({ dx: -72, dy: 12, viewState: "fit", inDrawer: false }))
  .toBe("next");
expect(classifyViewerGesture({ dx: 18, dy: 90, viewState: "fit", inDrawer: true }))
  .toBe("drawerScroll");
expect(classifyViewerGesture({ dx: -72, dy: 12, viewState: "zoomed", inDrawer: false }))
  .toBe("pan");
expect(classifyViewerGesture({ dx: 4, dy: 3, viewState: "fit", inDrawer: false }))
  .toBe("tap");
```

Use a 48-pixel swipe threshold and require horizontal travel to exceed vertical travel by a factor of 1.25.

- [ ] **Step 2: Add failing viewport and browser coalescing tests**

Test the browser-free measurement preference in the unit suite:

```ts
expect(chooseViewerViewport(
  { width: 844, height: 390 },
  { width: 900, height: 500 },
)).toEqual({ width: 844, height: 390 });
expect(chooseViewerViewport(null, { width: 900, height: 500 }))
  .toEqual({ width: 900, height: 500 });
```

In `PhotoViewer.browser.test.tsx`, install a visual-viewport test double and verify one animation-frame update preserves state:

```ts
viewport.setSize(844, 390);
viewport.dispatchEvent(new Event("resize"));
window.dispatchEvent(new Event("orientationchange"));
expect(result.current.revision).toBe(before);
flushAnimationFrame();
expect(result.current).toMatchObject({ width: 844, height: 390, revision: before + 1 });
```

Add a browser flow which opens photo B with its screen preview and drawer open, changes from 390 × 844 to 844 × 390, then asserts the same asset, screen-preview layer, drawer, and selected filmstrip button remain.

- [ ] **Step 3: Run focused tests and verify RED**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/useViewerGestures.test.ts src/viewer/viewerViewport.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
```

Expected: FAIL because gesture arbitration and visual-viewport relayout do not exist.

- [ ] **Step 4: Implement gesture arbitration**

Capture one primary pointer at a time. Record start position, pointer type, whether the gesture began in the drawer, and the current `viewState`. On release, classify the movement. Route `previous` or `next` through the existing navigation action. Route `tap` to touch-control toggling only. Leave `pan` as a consumed no-navigation result so the later zoom feature can supply movement without changing the interface.

Do not begin stage navigation from buttons, filmstrip items, or vertically scrolling drawer content. Cancel the active gesture on pointer cancellation, lost capture, viewport revision, and viewer close.

- [ ] **Step 5: Implement coalesced resize and rotation layout**

Measure `window.visualViewport?.width` and height, falling back to `document.documentElement.clientWidth` and height. Listen to visual viewport resize and scroll, window resize, and orientation change. Coalesce all signals through one animation frame and increment `revision` only when usable dimensions change.

Pass measured dimensions into the fixed-stage fit calculation. Preserve the current `<img>` elements and change only layout bounds. On revision, recalculate the drawer width, filmstrip capacity, safe-area offsets, and current filmstrip centring. Do not dispatch select, close, open, or preview-reset actions.

- [ ] **Step 6: Add safe-area and reduced-motion styling**

Use `max(44px, env(safe-area-inset-* ))` where controls need protected touch space. Use `100dvh` and measured CSS custom properties for the overlay rather than `100vh`. Ensure portrait and landscape layouts keep the back, Info, previous, next, drawer close, and filmstrip controls reachable. Disable stage, drawer, and filmstrip animation in the reduced-motion media query.

- [ ] **Step 7: Verify and commit Task 7**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/useViewerGestures.test.ts src/viewer/viewerViewport.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
npm test
npm run typecheck
npm run check
```

Then commit:

```bash
git add apps/interface/src
git commit -m "feat: support viewer swipe and device rotation"
```

- [ ] **Step 8: Demonstrate checkpoint 4**

Use phone-sized WebKit to demonstrate tap-controlled chrome, explicit Info opening, horizontal swipe, drawer vertical scroll, portrait-to-landscape rotation, and landscape-to-portrait rotation. Confirm the same photo and preview remain. Then run the macOS app and resize the window through narrow and wide layouts. Record both results.

---

### Task 8: Complete accessibility, offline evidence, and native acceptance

**Files:**
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/components/App.browser.test.tsx` if viewer-level application integration needs coverage
- Modify: `README.md`
- Create: `docs/superpowers/verification/2026-08-26-immersive-filmstrip-viewer.md`

**Interfaces:**
- Consumes: Tasks 1 through 7 and the existing controlled source fixtures.
- Produces: complete automated evidence, macOS exploratory build, source-read-only audit, and user-facing run instructions.

- [ ] **Step 1: Add final accessibility and offline browser assertions**

Run Axe against the open viewer, open information drawer, first boundary, last loaded boundary, and rotated phone layout. Assert no serious violations. Verify all controls are at least 44 by 44 CSS pixels at phone width, hidden controls do not retain focus, current filmstrip item has `aria-current`, and disabled directions expose their state.

Simulate a source-unavailable update after both thumbnail and screen-preview references are ready. Assert the viewer continues to render cached derivative URLs and navigate cached neighbours without exposing a native path.

- [ ] **Step 2: Run the complete interface suite**

Run:

```bash
npm test
npm run test:browser
npm run typecheck
npm run check
npm run --workspace @photo-viewer/interface build
```

Expected: all unit and WebKit tests, accessibility checks, type checks, formatting checks, and the production interface build pass.

- [ ] **Step 3: Run the complete Rust and desktop suite**

Run:

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check
cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-viewer.json
npm run desktop:build -- --bundles app
```

Expected: every workspace and desktop test passes, Clippy reports no warnings, formatting is clean, the benchmark smoke completes, and the unsigned macOS app bundle is created.

- [ ] **Step 4: Audit the source-media invariant**

Hash the controlled source fixture directory before and after native derivative generation and compare the manifests. Inspect the production diff for source-path writes, rename, move, copy, rating, tagging, and delete operations. Record that SQLite and managed cache paths are the only writes. Never record the user's selected native path.

- [ ] **Step 5: Run the macOS acceptance demonstration**

Launch with a new named profile and a real controlled folder. Demonstrate:

1. Immediate viewer paint from an existing thumbnail
2. Stable crossfade to the screen preview
3. Keyboard, button, filmstrip, and swipe navigation
4. No wrapping and continued pagination
5. Explicit information drawer and metadata updates
6. Pointer and touch control fading
7. Narrow, wide, portrait, and landscape relayout
8. Exact return to wall scroll and tile focus
9. Cached navigation after the source becomes unavailable

Leave the app running for the user's exploratory check. Do not merge until the user accepts the slice.

- [ ] **Step 6: Document and commit final evidence**

Update `README.md` with the viewer controls and clean-profile demonstration command. Record exact command results, test counts, build location, fixture-hash equality, limitations, and native observations in the verification document.

Run `git diff --check` and confirm only planned files changed. Commit:

```bash
git add README.md docs/superpowers/verification/2026-08-26-immersive-filmstrip-viewer.md apps/interface/src/components/PhotoViewer.browser.test.tsx apps/interface/src/components/App.browser.test.tsx
git commit -m "test: verify immersive filmstrip viewer"
```

Do not merge or push. Report the branch and running-app state for user acceptance.
