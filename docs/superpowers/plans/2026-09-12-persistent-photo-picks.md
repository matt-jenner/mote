# Persistent Photo Picks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persistent, cross-folder Picks collection that can be reviewed in the existing viewer, copied as original files in the desktop app, and optionally downloaded one at a time in hosted Mote.

**Architecture:** The interface owns one host-neutral pick-list model. Desktop mutations are persisted in SQLite and original copying stays behind Tauri commands; hosted mutations are persisted in browser `localStorage`, while the server only resolves catalog assets, generates derivatives, and conditionally streams originals. The current wall selection remains independent from the pick sequence, and every backend path operation re-authorizes catalog membership and source containment.

**Tech Stack:** React 19, TypeScript 7, CSS modules, Vitest browser/unit projects, Tauri 2, Rust 2024, Axum 0.8, SQLite/rusqlite.

**Spec:** [`docs/superpowers/specs/2026-09-12-persistent-photo-picks-design.md`](../specs/2026-09-12-persistent-photo-picks-design.md)

## Global Constraints

- Preserve the existing justified-wall layout, viewer gestures, warning badge placement, theme tokens, and source-folder behavior.
- Stable asset IDs are the membership identity. A pick may additionally retain an opaque source-folder ID and a label snapshot; browser storage must never contain an absolute path.
- Keep the desktop source tree read-only. Do not pass original paths to the webview.
- Keep hosted picks out of SQLite and all other server-side user state.
- Original downloads default to disabled and are enforced server-side, regardless of whether the interface renders a link.
- Never overwrite copied files. Treat names case-insensitively when reserving a destination filename, even on a case-sensitive filesystem.
- Keep unrelated worktree changes out of every commit. Stage only the files listed by the active task.
- Use the approved visual references for hierarchy and responsive behavior:
  - [`desktop`](../specs/assets/2026-09-12-pick-list-desktop-selected.png)
  - [`mobile browse`](../specs/assets/2026-09-12-pick-list-mobile-browse.png)
  - [`mobile sheet`](../specs/assets/2026-09-12-pick-list-mobile-sheet.png)

---

## Task 1: Add ordered desktop pick persistence to the catalog

**Files:**

- Create: `crates/catalog/migrations/0013_photo_picks.sql`
- Create: `crates/catalog/src/pick_repo.rs`
- Create: `crates/catalog/tests/photo_picks.rs`
- Modify: `crates/catalog/src/lib.rs`
- Modify: `crates/catalog/src/migrate.rs`

- [ ] **Step 1: Write the failing catalog tests**

Cover insertion order, duplicate add as a no-op, removal, immediate clear, undo merge, revision increments, pick survival after deleting a saved-folder shortcut, and `NativePathKey` round-tripping for the last successful copy destination.

```rust
#[test]
fn picks_are_ordered_unique_and_independent_of_saved_shortcuts() {
    let mut fixture = Fixture::new();
    let first = fixture.asset("one.jpg");
    let second = fixture.asset("two.jpg");

    assert!(fixture.catalog.add_photo_pick(first, fixture.group).unwrap());
    assert!(fixture.catalog.add_photo_pick(second, fixture.group).unwrap());
    assert!(!fixture.catalog.add_photo_pick(first, fixture.group).unwrap());
    fixture.catalog.remove_saved_folder(fixture.saved_id).unwrap();

    assert_eq!(
        fixture.catalog.list_photo_picks().unwrap()
            .into_iter().map(|pick| pick.asset_id).collect::<Vec<_>>(),
        vec![first, second],
    );
}
```

- [ ] **Step 2: Run the focused test and confirm it fails**

Run: `cargo test -p photo-catalog --test photo_picks`

Expected: failure because migration 13 and the pick repository API do not exist.

- [ ] **Step 3: Add migration 13**

Use a single preferences row for mutation revision and the remembered native destination. Keep the source group after a saved shortcut is removed; cascade only if the underlying catalog asset or group is actually deleted.

```sql
CREATE TABLE photo_picks (
    position INTEGER PRIMARY KEY,
    asset_id BLOB NOT NULL UNIQUE REFERENCES assets(id) ON DELETE CASCADE,
    folder_group_id BLOB NOT NULL REFERENCES folder_groups(id) ON DELETE CASCADE
);

CREATE TABLE photo_pick_preferences (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    last_copy_destination BLOB
);

INSERT INTO photo_pick_preferences (singleton) VALUES (1);
PRAGMA user_version = 13;
```

- [ ] **Step 4: Implement transactional repository operations**

Expose:

```rust
pub struct PhotoPickRecord {
    pub asset_id: AssetId,
    pub folder_group_id: FolderGroupId,
    pub position: u64,
}

impl Catalog {
    pub fn photo_pick_revision(&self) -> Result<u64, CatalogError>;
    pub fn list_photo_picks(&self) -> Result<Vec<PhotoPickRecord>, CatalogError>;
    pub fn add_photo_pick(&mut self, asset: AssetId, group: FolderGroupId) -> Result<bool, CatalogError>;
    pub fn remove_photo_pick(&mut self, asset: AssetId) -> Result<bool, CatalogError>;
    pub fn clear_photo_picks(&mut self) -> Result<Vec<PhotoPickRecord>, CatalogError>;
    pub fn restore_photo_picks(&mut self, cleared: &[PhotoPickRecord]) -> Result<(), CatalogError>;
    pub fn last_copy_destination(&self) -> Result<Option<NativePathKey>, CatalogError>;
    pub fn set_last_copy_destination(&mut self, value: &NativePathKey) -> Result<(), CatalogError>;
}
```

`restore_photo_picks` must atomically produce `cleared unique entries in their old order + picks added since clear in their new order`, so Undo never discards a post-clear selection.

- [ ] **Step 5: Run catalog tests**

Run: `cargo test -p photo-catalog --test photo_picks && cargo test -p photo-catalog`

Expected: all catalog tests pass and databases at schema 12 migrate once to schema 13.

- [ ] **Step 6: Commit**

```bash
git add crates/catalog/migrations/0013_photo_picks.sql crates/catalog/src/pick_repo.rs crates/catalog/src/lib.rs crates/catalog/src/migrate.rs crates/catalog/tests/photo_picks.rs
git commit -m "feat(catalog): persist ordered photo picks"
```

---

## Task 2: Add pick DTOs, authorization, and hydration to app-service

**Files:**

- Create: `crates/app-service/src/picks.rs`
- Create: `crates/app-service/tests/photo_picks.rs`
- Modify: `crates/app-service/src/dto.rs`
- Modify: `crates/app-service/src/lib.rs`
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/app-service/src/gallery.rs`

- [ ] **Step 1: Write failing service tests**

Test that desktop picks hydrate across multiple folder groups without changing the active selection, preserve insertion order, return cached derivative references, keep an unavailable asset, reject an asset that does not belong to the supplied group, and request pick derivatives with `IncludeSubfolders` authorization.

```rust
#[tokio::test]
async fn resolving_picks_does_not_change_the_active_wall() {
    let fixture = Fixture::with_two_saved_folders().await;
    let before = fixture.service.bootstrap().unwrap().active_source;
    let picks = fixture.service.list_photo_picks().unwrap();

    assert_eq!(picks.items.len(), 2);
    assert_eq!(fixture.service.bootstrap().unwrap().active_source, before);
}
```

- [ ] **Step 2: Run the focused test and confirm it fails**

Run: `cargo test -p photo-app-service --test photo_picks`

Expected: failure because the pick DTO and service methods are absent.

- [ ] **Step 3: Define the host-neutral wire model**

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickReference {
    pub asset_id: String,
    pub source_folder_id: String,
    pub source_label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickItem {
    #[serde(flatten)]
    pub reference: PickReference,
    pub asset: Option<WallAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickListSnapshot {
    pub revision: u64,
    pub items: Vec<PickItem>,
}
```

Bound request batches to 250 IDs, reject malformed UUIDs, and expose no native paths.

- [ ] **Step 4: Implement desktop mutations and hydration**

Add `AppService::{list,add,remove,clear,restore}_photo_picks` around the catalog repository. `add_photo_pick` accepts an asset ID and opaque folder-group ID, proves membership with `wall_records_for_assets_scoped(group, IncludeSubfolders, &[asset])`, and then inserts. Hydration reuses `wall_assets_with_derivatives` and derives a source label from the saved custom label when present, otherwise the folder-group display name.

- [ ] **Step 5: Implement hosted selection-scoped resolution**

Add `GalleryEngine::resolve_assets(&GallerySelection, &[String]) -> Vec<Option<WallAsset>>`. Preserve request order, validate that every returned asset belongs to the hosted library and selection group, and return `None` for a catalog ID that has genuinely disappeared. Do not start a scan and do not mutate active selection.

- [ ] **Step 6: Add cross-folder derivative requests**

Add `AppService::request_pick_derivatives` for persisted desktop picks. Group authorized IDs by folder group and use `GalleryScope::IncludeSubfolders`; do not use the current gallery scope or current active folder.

- [ ] **Step 7: Run the service suite**

Run: `cargo test -p photo-app-service --test photo_picks && cargo test -p photo-app-service`

Expected: focused and regression tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/app-service/src/picks.rs crates/app-service/src/dto.rs crates/app-service/src/lib.rs crates/app-service/src/service.rs crates/app-service/src/gallery.rs crates/app-service/tests/photo_picks.rs
git commit -m "feat(app-service): authorize and hydrate photo picks"
```

---

## Task 3: Build the browser-only pick store

**Files:**

- Create: `apps/interface/src/picks/browserPicks.ts`
- Create: `apps/interface/src/picks/browserPicks.test.ts`
- Create: `apps/interface/src/picks/pickList.ts`
- Create: `apps/interface/src/picks/pickList.test.ts`

- [ ] **Step 1: Write failing storage and reducer tests**

Cover ordering, duplicate adds, remove, clear, undo merge, malformed-record recovery, root scoping, storage-event reloads, a failed `localStorage` write falling back to in-memory state, and no path-like fields in serialized records.

```ts
expect(JSON.parse(storage.getItem("mote.picks.v1.root-a") ?? "null")).toEqual({
  revision: 2,
  items: [
    { assetId: "asset-1", sourceFolderId: "folder-1", sourceLabel: "Family" },
  ],
});
expect(storage.getItem("mote.picks.v1.root-b")).toBeNull();
```

- [ ] **Step 2: Run focused tests and confirm they fail**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/picks/browserPicks.test.ts src/picks/pickList.test.ts`

Expected: failure because the pick store does not exist.

- [ ] **Step 3: Implement the shared reference model**

```ts
export interface PickReference {
  assetId: string;
  sourceFolderId: string;
  sourceLabel: string;
}

export interface PickItem extends PickReference {
  asset: WallAsset | null;
}

export interface PickListSnapshot {
  revision: number;
  items: PickItem[];
  persistenceError: string | null;
}
```

Use pure helpers for `add`, `remove`, `clear`, and `restoreCleared`. `restoreCleared` prepends the cleared unique items in their former order and appends any selections made after the clear.

- [ ] **Step 4: Implement root-scoped localStorage**

Use the key `mote.picks.v1.${encodeURIComponent(rootId)}`. Persist only `PickReference[]` and a revision. Ignore malformed entries individually, publish storage failures, and expose `read`, `subscribe`, `add`, `remove`, `clear`, `restore`, and `handleStorageEvent`.

- [ ] **Step 5: Run tests**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/picks/browserPicks.test.ts src/picks/pickList.test.ts`

Expected: all focused tests pass.

- [ ] **Step 6: Commit**

```bash
git add apps/interface/src/picks/browserPicks.ts apps/interface/src/picks/browserPicks.test.ts apps/interface/src/picks/pickList.ts apps/interface/src/picks/pickList.test.ts
git commit -m "feat(interface): add browser-local pick storage"
```

---

## Task 4: Extend PhotoService and both host adapters

**Files:**

- Modify: `apps/interface/src/services/photoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.test.ts`
- Modify: `apps/interface/src/services/httpPhotoService.ts`
- Modify: `apps/interface/src/services/httpPhotoService.test.ts`
- Modify: `apps/interface/src/services/inMemoryPhotoService.ts`
- Create: `crates/server/tests/pick_api.rs`
- Modify: `crates/server/src/api/types.rs`
- Modify: `crates/server/src/api/gallery.rs`
- Modify: `crates/server/src/api/mod.rs`
- Modify: `crates/server/src/lib.rs`

- [ ] **Step 1: Write failing adapter and API tests**

Verify:

- Tauri invokes `list_photo_picks`, mutation commands, and `request_pick_derivatives` with bounded IDs.
- HTTP initializes the root-scoped browser store only after bootstrap supplies `rootId`.
- HTTP groups resolution and derivative calls by `sourceFolderId` and uses `selection-${sourceFolderId}` without changing active selection.
- A `storage` event refreshes another tab's in-memory snapshot.
- `POST /api/v1/selections/{id}/assets` rejects foreign IDs and preserves the requested order.

- [ ] **Step 2: Run focused tests and confirm they fail**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/tauriPhotoService.test.ts src/services/httpPhotoService.test.ts
cargo test -p photo-server --test pick_api
```

Expected: failures for missing capabilities and routes.

- [ ] **Step 3: Extend the service contract**

```ts
export interface PhotoServiceCapabilities {
  chooseFolder: boolean;
  folderSelection: "native" | "hosted";
  locateFolder: boolean;
  originalAction: "copy" | "download" | "none";
}

export interface PhotoService {
  // existing methods...
  getPicks(): PickListSnapshot;
  watchPicks(listener: (snapshot: PickListSnapshot) => void): () => void;
  loadPicks(): Promise<PickListSnapshot>;
  addPick(reference: PickReference): Promise<PickListSnapshot>;
  removePick(assetId: string): Promise<PickListSnapshot>;
  clearPicks(): Promise<PickListSnapshot>;
  restorePicks(cleared: readonly PickReference[]): Promise<PickListSnapshot>;
  requestPickDerivatives(request: DerivativeRequest): Promise<void>;
  originalDownloadUrl(assetId: string): string | null;
}
```

Desktop reports `copy`; hosted reports `none` for now (Task 11 switches it from bootstrap capability); memory mode uses an in-memory implementation.

- [ ] **Step 4: Add the selection asset endpoint**

Define a bounded request:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveAssetsRequest {
    pub asset_ids: Vec<String>,
}
```

Add `POST /api/v1/selections/{id}/assets`. It resolves the selection, delegates to `GalleryEngine::resolve_assets`, returns a path-free ordered array, and uses the existing safe error envelope.

- [ ] **Step 5: Implement the adapters**

Tauri delegates to native commands and publishes returned revisions. HTTP owns `createBrowserPicks`, hydrates each folder group through the new endpoint, merges the ordered response back into browser references, and carries `sourceLabel` from local storage rather than trusting a server path.

- [ ] **Step 6: Run adapter, server, type, and regression tests**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/tauriPhotoService.test.ts src/services/httpPhotoService.test.ts
npm run typecheck
cargo test -p photo-server --test pick_api
cargo test -p photo-server
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add apps/interface/src/services/photoService.ts apps/interface/src/services/tauriPhotoService.ts apps/interface/src/services/tauriPhotoService.test.ts apps/interface/src/services/httpPhotoService.ts apps/interface/src/services/httpPhotoService.test.ts apps/interface/src/services/inMemoryPhotoService.ts crates/server/src/api/types.rs crates/server/src/api/gallery.rs crates/server/src/api/mod.rs crates/server/src/lib.rs crates/server/tests/pick_api.rs
git commit -m "feat: connect pick storage to desktop and hosted services"
```

---

## Task 5: Add the shared pick controller, live announcements, and clear Undo

**Files:**

- Create: `apps/interface/src/picks/PickListContext.tsx`
- Create: `apps/interface/src/picks/usePickList.ts`
- Create: `apps/interface/src/picks/usePickList.test.tsx`
- Create: `apps/interface/src/components/PickToast.tsx`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/styles/appShell.module.css`

- [ ] **Step 1: Write failing hook tests**

Test initial load, synchronous `isPicked`, duplicate-toggle suppression while a mutation is pending, optimistic count updates with rollback on error, five-second clear Undo, Undo merge, storage warning exposure, and cleanup of service subscriptions/timers.

- [ ] **Step 2: Run the hook test and confirm it fails**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/picks/usePickList.test.tsx`

Expected: failure because the controller and context are missing.

- [ ] **Step 3: Implement one shared controller**

```ts
export interface PickListController {
  snapshot: PickListSnapshot;
  count: number;
  isPicked(assetId: string): boolean;
  toggle(asset: WallAsset, origin: PickOrigin): Promise<void>;
  remove(assetId: string): Promise<void>;
  clear(): Promise<void>;
  undoClear(): Promise<void>;
  requestDerivatives(assetIds: readonly string[], kind: DerivativeClass): void;
}
```

The hook is the only owner of pick mutation state. Add a polite live region for `Added to picks`, `Removed from picks`, `Picks cleared`, Undo, persistence warnings, and later copy progress. Keep toast action focusable without moving focus into it.

- [ ] **Step 4: Mount the provider in AppShell**

Derive `PickOrigin` from the active saved-folder entry (`folderId` plus `folderLabel`). Keep the provider outside wall/viewer conditional branches so folder switches cannot reset it.

- [ ] **Step 5: Run tests and typecheck**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/picks/usePickList.test.tsx && npm run typecheck`

Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add apps/interface/src/picks/PickListContext.tsx apps/interface/src/picks/usePickList.ts apps/interface/src/picks/usePickList.test.tsx apps/interface/src/components/PickToast.tsx apps/interface/src/components/AppShell.tsx apps/interface/src/styles/appShell.module.css
git commit -m "feat(interface): add shared pick list controller"
```

---

## Task 6: Add accessible pick toggles to the wall and viewer

**Files:**

- Modify: `apps/interface/src/components/PhotoTile.tsx`
- Modify: `apps/interface/src/components/JustifiedWall.tsx`
- Modify: `apps/interface/src/components/PhotoWallCanvas.tsx`
- Modify: `apps/interface/src/components/SourceCanvas.tsx`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/styles/photoWall.module.css`
- Modify: `apps/interface/src/styles/photoViewer.module.css`
- Modify: `apps/interface/src/components/PhotoWall.browser.test.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`

- [ ] **Step 1: Write failing browser tests**

Verify the tile toggle is distinct from the open overlay, uses `aria-pressed`, has a filename-specific accessible name, remains visible on touch, does not collide with the top-right warning badge, updates matching wall/viewer instances immediately, and viewer chrome exposes the same toggle.

- [ ] **Step 2: Run the browser tests and confirm they fail**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PhotoWall.browser.test.tsx src/components/PhotoViewer.browser.test.tsx`

Expected: assertions cannot find pick controls.

- [ ] **Step 3: Add wall toggles**

Use the icon library already in the app; do not draw a custom checkmark asset. Stop propagation on the toggle, keep the open overlay separate, and layer controls as:

```text
top-left: pick toggle
top-right: existing warning badge
full tile: existing open overlay
```

Selected tiles retain a quiet green outline in light and dark themes. Unselected controls can fade until hover/focus on precise pointers, but remain visible under `(hover: none), (pointer: coarse)`.

- [ ] **Step 4: Add viewer toggle**

Place `Add {filename} to picks` / `Remove {filename} from picks` beside existing chrome controls. It must participate in the viewer's existing controls-visible and focus-trap behavior.

- [ ] **Step 5: Run browser tests, contrast checks, and typecheck**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PhotoWall.browser.test.tsx src/components/PhotoViewer.browser.test.tsx src/components/PhotoViewer.contrast.test.tsx
npm run typecheck
```

Expected: pass with no existing viewer regression.

- [ ] **Step 6: Commit**

```bash
git add apps/interface/src/components/PhotoTile.tsx apps/interface/src/components/JustifiedWall.tsx apps/interface/src/components/PhotoWallCanvas.tsx apps/interface/src/components/SourceCanvas.tsx apps/interface/src/components/PhotoViewerOverlay.tsx apps/interface/src/styles/photoWall.module.css apps/interface/src/styles/photoViewer.module.css apps/interface/src/components/PhotoWall.browser.test.tsx apps/interface/src/components/PhotoViewer.browser.test.tsx
git commit -m "feat(interface): pick photos from wall and viewer"
```

---

## Task 7: Build the responsive Picks drawer and mobile sheet

**Files:**

- Create: `apps/interface/src/components/PicksPanel.tsx`
- Create: `apps/interface/src/components/PickRow.tsx`
- Create: `apps/interface/src/styles/picksPanel.module.css`
- Create: `apps/interface/src/components/PicksPanel.browser.test.tsx`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/styles/appShell.module.css`

- [ ] **Step 1: Write failing interaction and layout tests**

At desktop width verify toolbar toggle/count, `aria-expanded`, X/Escape close, non-modal focus behavior, no clear-on-close, ~320px docked width, wall reflow, sticky footer, empty state, row removal, and source warnings. At 390px verify the 56px safe-area-aware bar, 84% modal sheet, focus trap/restore, backdrop/X/Escape/platform-back close, and no final-row occlusion.

- [ ] **Step 2: Run the browser test and confirm it fails**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PicksPanel.browser.test.tsx`

Expected: panel controls are absent.

- [ ] **Step 3: Implement the shared panel contents**

Render a thumbnail, filename, retained source label, warning state, remove control, optional row action slot, and sticky actions. The empty state shows selection guidance and hides Review, Clear, Copy, and download controls.

- [ ] **Step 4: Implement desktop dock behavior**

At `min-width: 900px`, add a right grid column near 320px. The wall remains interactive and remeasures as the workspace narrows. Start every launch closed; X, Escape, and the toolbar button close it without touching picks.

- [ ] **Step 5: Implement mobile sheet behavior**

At `max-width: 899px`, render the persistent bottom bar and modal sheet. Add `padding-bottom: calc(56px + env(safe-area-inset-bottom))` to the browsing canvas while the bar is present. Trap focus only for the modal sheet, make the background inert, and treat a downward drag beyond the threshold as dismissal.

- [ ] **Step 6: Implement immediate Clear and Undo**

`Clear picks` calls the shared controller immediately, closes no surfaces, and publishes the Undo toast. It never opens a confirmation dialog.

- [ ] **Step 7: Run browser tests, reduced-motion tests, and typecheck**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PicksPanel.browser.test.tsx src/components/App.browser.test.tsx src/components/PhotoViewer.motion.test.tsx
npm run typecheck
```

Expected: pass.

- [ ] **Step 8: Commit**

```bash
git add apps/interface/src/components/PicksPanel.tsx apps/interface/src/components/PickRow.tsx apps/interface/src/styles/picksPanel.module.css apps/interface/src/components/PicksPanel.browser.test.tsx apps/interface/src/components/AppShell.tsx apps/interface/src/styles/appShell.module.css
git commit -m "feat(interface): add responsive Picks panel"
```

---

## Task 8: Add pick-only immersive review across folders

**Files:**

- Create: `apps/interface/src/viewer/pickSequence.ts`
- Create: `apps/interface/src/viewer/pickSequence.test.ts`
- Modify: `apps/interface/src/viewer/viewerReducer.ts`
- Modify: `apps/interface/src/viewer/viewerReducer.test.ts`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/components/PicksPanel.tsx`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/ViewerFilmstrip.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`

- [ ] **Step 1: Write failing sequence and browser tests**

Test pick insertion order across folders, `Picks · 3 of 6`, previous/next/keyboard/swipe/filmstrip limited to picks, derivative requests independent of active wall, row-thumbnail entry, close restoring panel and wall scroll, current-item removal advancing next or previous, and removing the sole item returning to the empty panel.

- [ ] **Step 2: Run focused tests and confirm they fail**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/pickSequence.test.ts src/viewer/viewerReducer.test.ts
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PhotoViewer.browser.test.tsx
```

Expected: review mode is not represented.

- [ ] **Step 3: Model viewer sequence and return surface**

```ts
export type ViewerSequence = "wall" | "picks";
export type ViewerReturnSurface = "wall" | "picksPanel";

// Add to ViewerState:
sequence: ViewerSequence;
returnSurface: ViewerReturnSurface;
```

Wall open uses `wall`; `Review picks` and a row thumbnail use `picks`. Do not alter the active source or the wall reducer.

- [ ] **Step 4: Feed the viewer the selected sequence**

For pick review, build `assets` from hydrated pick items in insertion order, set `nextCursor` to `null`, disable wall pagination, and route preview requests through `requestPickDerivatives`. Keep unavailable assets with cached derivatives; keep asset-less stale rows in the panel but do not fabricate image dimensions to open them.

- [ ] **Step 5: Handle removal deterministically**

Before removing the current review item, compute the next ID from the old list: next item when available, otherwise previous. Close only when no reviewable picks remain. The viewer pick toggle and panel row remove must share this rule.

- [ ] **Step 6: Restore surface, focus, and scroll**

Closing pick review reopens the previously launched desktop drawer or mobile sheet and restores focus to `Review picks` or the launching row when it still exists. Preserve the browsing folder's wall scroll throughout.

- [ ] **Step 7: Run unit/browser tests and typecheck**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/pickSequence.test.ts src/viewer/viewerReducer.test.ts
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PhotoViewer.browser.test.tsx src/components/PicksPanel.browser.test.tsx
npm run typecheck
```

Expected: pass.

- [ ] **Step 8: Commit**

```bash
git add apps/interface/src/viewer/pickSequence.ts apps/interface/src/viewer/pickSequence.test.ts apps/interface/src/viewer/viewerReducer.ts apps/interface/src/viewer/viewerReducer.test.ts apps/interface/src/components/AppShell.tsx apps/interface/src/components/PicksPanel.tsx apps/interface/src/components/PhotoViewerOverlay.tsx apps/interface/src/components/ViewerFilmstrip.tsx apps/interface/src/components/PhotoViewer.browser.test.tsx
git commit -m "feat(interface): review picks across folders"
```

---

## Task 9: Implement safe, collision-free original copying

**Files:**

- Create: `crates/app-service/src/original_copy.rs`
- Create: `crates/app-service/tests/original_copy.rs`
- Modify: `crates/app-service/src/lib.rs`
- Modify: `crates/app-service/src/service.rs`

- [ ] **Step 1: Write failing filesystem tests**

Use temporary source and destination trees. Cover flat copy, byte equality, unchanged source hash/metadata, numeric suffixes, case-insensitive collision reservation, collisions within the same batch, missing/unreadable sources, destination loss midway, retry, source-root rejection for every configured library, and an immutable batch when picks mutate after preparation.

```rust
assert_eq!(names(&destination), ["IMG_2048.jpg", "IMG_2048 (2).jpg"]);
assert_eq!(sha256(&source), source_hash_before);
assert_eq!(source.metadata().unwrap().modified().unwrap(), modified_before);
```

- [ ] **Step 2: Run the focused test and confirm it fails**

Run: `cargo test -p photo-app-service --test original_copy`

Expected: copy API is absent.

- [ ] **Step 3: Define immutable batch and safe result types**

```rust
pub struct OriginalCopyBatch { /* private validated snapshot */ }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyItemResult {
    pub asset_id: String,
    pub status: CopyItemStatus,
    pub destination_name: Option<String>,
    pub error_code: Option<String>,
}
```

Expose only IDs, destination filenames, counts, and bounded error codes to the interface. Never serialize a source path.

- [ ] **Step 4: Prepare and validate the batch**

`AppService::prepare_original_copy(asset_ids)` snapshots current pick records, verifies each requested ID is still a pick, resolves its library root plus typed relative path, and records per-item unavailable failures without aborting the whole batch. Canonicalize the destination and reject it when equal to or beneath any configured source root.

- [ ] **Step 5: Copy without overwrite races**

Scan existing destination names into a case-folded reservation set. Generate `name`, `name (2)`, ... before the extension, and create the destination with `OpenOptions::create_new(true)`. If another process wins the race, reserve the next suffix. Stream bytes; remove only the incomplete destination file on a write failure. Never modify or remove the source.

- [ ] **Step 6: Persist the remembered destination**

Update `last_copy_destination` only after a completed operation copies at least one file. Cancellation never reaches this method and therefore never changes it.

- [ ] **Step 7: Run focused and full service tests**

Run: `cargo test -p photo-app-service --test original_copy && cargo test -p photo-app-service`

Expected: pass.

- [ ] **Step 8: Commit**

```bash
git add crates/app-service/src/original_copy.rs crates/app-service/src/lib.rs crates/app-service/src/service.rs crates/app-service/tests/original_copy.rs
git commit -m "feat(app-service): copy picked originals safely"
```

---

## Task 10: Bridge native copy, progress, retry, and Show folder into the UI

**Files:**

- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/interface/src/services/photoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.test.ts`
- Modify: `apps/interface/src/picks/usePickList.ts`
- Modify: `apps/interface/src/components/PicksPanel.tsx`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/components/PicksPanel.browser.test.tsx`

- [ ] **Step 1: Write failing native and interface tests**

Test native cancel, picker initial directory, one-operation lock, progress order, completion, partial result, failed-only retry, drawer close not cancelling, remembered destination after successful copy, and Show folder. Test that clear/remove during copy changes saved picks but not the running batch.

- [ ] **Step 2: Run focused tests and confirm they fail**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/tauriPhotoService.test.ts
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PicksPanel.browser.test.tsx
```

Expected: native commands and copy UI are missing.

- [ ] **Step 3: Add native commands and one-operation lock**

Add an `Arc<tokio::sync::Mutex<()>>` to `DesktopState` and acquire it with `try_lock_owned` before opening the picker. Add:

```rust
#[tauri::command]
pub async fn copy_picked_originals(
    app: AppHandle,
    asset_ids: Option<Vec<String>>,
    on_event: tauri::ipc::Channel<CopyProgress>,
    state: State<'_, DesktopState>,
) -> Result<CopyResult, CommandError>;

#[tauri::command]
pub fn show_last_copy_destination(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError>;
```

Initialize the destination dialog with the stored destination when it still exists. Use the opener plugin from Rust for Show folder. Register commands and plugin in `lib.rs`.

- [ ] **Step 4: Extend the TypeScript service**

Add `copyPickedOriginals(assetIds, listener)` and `showLastCopyDestination()` to `PhotoService`. The HTTP and memory adapters return `unsupportedCapability`; the Tauri adapter creates a typed `Channel<CopyProgress>` and maps only bounded error codes.

- [ ] **Step 5: Add global copy state and UI**

The shared pick controller owns `idle | choosing | copying | complete | partial`. Show determinate `completed / total` on both the Picks toolbar button and panel footer. Keep it visible when the drawer is closed. A partial result marks failed rows and changes the primary action to `Retry N originals...`; retry sends only failed asset IDs and opens the picker again.

- [ ] **Step 6: Add completion behavior**

Show `Copied N originals` or `Copied X of N` in the live region and toast. Offer Show folder when at least one file copied. Never clear picks. Do not roll back successful files.

- [ ] **Step 7: Run native, interface, and regression tests**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/tauriPhotoService.test.ts
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PicksPanel.browser.test.tsx
npm run typecheck
```

Expected: pass.

- [ ] **Step 8: Commit**

```bash
git add apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/src/dto.rs apps/desktop/src-tauri/src/state.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src-tauri/src/lib.rs apps/interface/src/services/photoService.ts apps/interface/src/services/tauriPhotoService.ts apps/interface/src/services/tauriPhotoService.test.ts apps/interface/src/picks/usePickList.ts apps/interface/src/components/PicksPanel.tsx apps/interface/src/components/AppShell.tsx apps/interface/src/components/PicksPanel.browser.test.tsx
git commit -m "feat(desktop): copy picked originals with progress"
```

---

## Task 11: Add hosted original-download configuration and capability negotiation

**Files:**

- Modify: `crates/server/src/config.rs`
- Modify: `crates/server/src/lib.rs`
- Modify: `crates/server/src/api/types.rs`
- Modify: `crates/server/src/api/mod.rs`
- Create: `crates/server/tests/original_download_config.rs`
- Modify: `apps/interface/src/services/httpPhotoService.ts`
- Modify: `apps/interface/src/services/httpPhotoService.test.ts`
- Modify: `README.md`

- [ ] **Step 1: Write failing config/bootstrap tests**

Test missing, empty, `0`, `false`, `1`, and `true`; accept ASCII case-insensitively and reject other non-empty values as configuration errors. Verify bootstrap emits `originalDownloads: false` by default and true only when enabled.

- [ ] **Step 2: Run focused tests and confirm they fail**

Run:

```bash
cargo test -p photo-server --test original_download_config
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/httpPhotoService.test.ts
```

Expected: bootstrap does not contain the capability.

- [ ] **Step 3: Parse the restart-time environment setting**

Add `allow_original_downloads: bool` to `ServerConfig`, default it to false in `ServerConfig::new`, and parse `PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS` only in `from_env`. Provide a test builder method instead of changing every existing constructor call.

- [ ] **Step 4: Advertise the capability**

```rust
pub struct Capabilities {
    pub folder_browser: bool,
    pub video: bool,
    pub original_downloads: bool,
}
```

Carry the value into `AppState`. The HTTP decoder must require the boolean and set `service.capabilities.originalAction` to `download` or `none`. It must never probe a download URL to infer capability.

- [ ] **Step 5: Document operation**

Add the variable to the hosted/Docker configuration table: default off, accepted values, and restart required after changing it.

- [ ] **Step 6: Run tests**

Run: `cargo test -p photo-server --test original_download_config && npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/httpPhotoService.test.ts`

Expected: pass.

- [ ] **Step 7: Commit**

```bash
git add crates/server/src/config.rs crates/server/src/lib.rs crates/server/src/api/types.rs crates/server/src/api/mod.rs crates/server/tests/original_download_config.rs apps/interface/src/services/httpPhotoService.ts apps/interface/src/services/httpPhotoService.test.ts README.md
git commit -m "feat(hosted): advertise original download capability"
```

---

## Task 12: Stream authorized hosted originals and render per-row links

**Files:**

- Create: `crates/server/src/api/original.rs`
- Create: `crates/server/tests/original_download_api.rs`
- Modify: `crates/server/src/api/mod.rs`
- Modify: `crates/server/src/lib.rs`
- Modify: `crates/server/src/static_host.rs`
- Modify: `crates/server/Cargo.toml`
- Modify: `Cargo.toml`
- Modify: `crates/app-service/src/gallery.rs`
- Modify: `apps/interface/src/services/httpPhotoService.ts`
- Modify: `apps/interface/src/components/PickRow.tsx`
- Modify: `apps/interface/src/components/PicksPanel.tsx`
- Modify: `apps/interface/src/components/PicksPanel.browser.test.tsx`

- [ ] **Step 1: Write failing server security tests**

Cover disabled direct requests, enabled success, foreign/malformed/missing IDs, unavailable and unreadable originals, encoded traversal attempts, symlinked files and directory escapes, mount/path replacement race where supported, content length, attachment disposition, non-ASCII filename encoding, and absence of absolute paths in every error body.

- [ ] **Step 2: Run focused tests and confirm they fail**

Run: `cargo test -p photo-server --test original_download_api`

Expected: route is absent.

- [ ] **Step 3: Resolve originals by catalog identity**

Add `GalleryEngine::resolve_hosted_original(asset_id)` returning a typed relative path, display filename, and media kind only after verifying the asset belongs to `hosted_library_id`. The public endpoint accepts only the catalog UUID.

- [ ] **Step 4: Open from the pinned source root**

Reuse the Unix descriptor-relative `PinnedDirectory` walk from static hosting. Add a narrow `try_clone`/`open_regular_file` API, retain a cloned source descriptor in `AppState`, reject symlinks with `O_NOFOLLOW`, verify the final descriptor is a regular file, and validate mount identity where the platform supports it. Keep a canonicalize-and-containment fallback only for platforms where hosted source serving is already disabled.

- [ ] **Step 5: Add the guarded route**

Register `GET /api/v1/originals/{assetId}`. Check `allow_original_downloads` before catalog lookup, open the validated relative file in blocking work, and stream with `tokio_util::io::ReaderStream`. Return `Content-Type: application/octet-stream`, exact `Content-Length`, `X-Content-Type-Options: nosniff`, and RFC 5987-compatible `Content-Disposition: attachment` without reflecting control characters.

- [ ] **Step 6: Render one hosted action per available row**

`HttpPhotoService.originalDownloadUrl` returns the encoded route only when capability is `download`. Each available pick row renders `Download original`; unavailable rows do not. With capability `none`, render no link or disabled download button and show one quiet panel note that the site does not offer original downloads. Review remains fully enabled.

- [ ] **Step 7: Run server, browser, and type tests**

Run:

```bash
cargo test -p photo-server --test original_download_api
cargo test -p photo-server
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PicksPanel.browser.test.tsx src/components/PhotoViewer.browser.test.tsx
npm run typecheck
```

Expected: pass; direct downloads remain unavailable while disabled.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/server/Cargo.toml crates/server/src/api/original.rs crates/server/src/api/mod.rs crates/server/src/lib.rs crates/server/src/static_host.rs crates/server/tests/original_download_api.rs crates/app-service/src/gallery.rs apps/interface/src/services/httpPhotoService.ts apps/interface/src/components/PickRow.tsx apps/interface/src/components/PicksPanel.tsx apps/interface/src/components/PicksPanel.browser.test.tsx
git commit -m "feat(hosted): securely download picked originals"
```

---

## Task 13: Finish integration coverage, accessibility, and visual QA

**Files:**

- Create: `tests/hosted/picks.spec.ts`
- Modify: `apps/interface/src/components/App.browser.test.tsx`
- Modify: `apps/interface/src/components/PicksPanel.browser.test.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.contrast.test.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.motion.test.tsx`
- Modify: `README.md`
- Modify: any feature file only when a failing verification exposes a defect

- [ ] **Step 1: Add the end-to-end hosted scenario**

Exercise two saved folders: add picks in each, reload, remove a saved shortcut, review the cross-folder sequence, clear and Undo, simulate a second-tab storage event, and verify downloads absent by default. Start a second hosted fixture with downloads enabled and verify only explicit single-row links download originals.

- [ ] **Step 2: Add final accessibility assertions**

Run axe checks for the wall, desktop drawer, mobile sheet, and review viewer. Assert 44px mobile targets, correct accessible names, polite live regions, focus restoration, sheet background inertness, drawer non-modality, and reduced-motion transition removal.

- [ ] **Step 3: Run the complete automated verification**

Run:

```bash
npm run check
npm run typecheck
npm test
npm run test:browser
npm run web:build
npm run test:hosted -- --grep Picks
cargo test --workspace
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Expected: every command exits 0. If an unrelated pre-existing failure appears, record the exact command/output and do not conceal it.

- [ ] **Step 4: Inspect the visuals at agreed breakpoints**

Run the local memory fixture and capture these states in both light and dark themes where applicable:

- 1440px desktop, drawer closed/open, empty/non-empty, copy progress/partial
- 768px narrow layout
- 390px phone, bottom bar, sheet, pick review
- warning badge plus selected pick control on the same tile
- reduced-motion mode

Compare side-by-side with the three approved visual references. Fix only measurable hierarchy, spacing, overlap, contrast, truncation, safe-area, or focus defects; do not redesign the approved direction.

- [ ] **Step 5: Self-review against the spec**

Search for unfinished work and unsafe leaks:

```bash
rg -n "TODO|FIXME|placeholder|not implemented" apps/interface crates apps/desktop/src-tauri
rg -n "canonical_root|display_path|source_root" apps/interface/src/picks apps/interface/src/services
git diff --check
```

Confirm every in-scope requirement has a test or an explicit manual verification note. Confirm type names and JSON field names match across Rust DTOs and TypeScript decoders.

- [ ] **Step 6: Update README user guidance**

Document Picks persistence differences, explicit Clear/Undo, desktop copy collision behavior, the remembered destination, hosted per-row downloads, and the default-off environment setting. Do not promise ZIP, sharing, named lists, or server sync.

- [ ] **Step 7: Request code review**

Use `superpowers:requesting-code-review` to inspect the complete branch against the approved spec. Address only verified issues, rerunning the relevant focused test after each fix.

- [ ] **Step 8: Run final verification after review fixes**

Run the full command set from Step 3 again and record the final passing outputs.

- [ ] **Step 9: Commit final integration work**

```bash
git add tests/hosted/picks.spec.ts apps/interface/src/components/App.browser.test.tsx apps/interface/src/components/PicksPanel.browser.test.tsx apps/interface/src/components/PhotoViewer.browser.test.tsx apps/interface/src/components/PhotoViewer.contrast.test.tsx apps/interface/src/components/PhotoViewer.motion.test.tsx README.md
git commit -m "test: verify persistent photo picks end to end"
```

- [ ] **Step 10: Prepare branch handoff**

Use `superpowers:finishing-a-development-branch` only after all verification passes. Keep the approved design spec, generated references, and this plan in the branch history.
