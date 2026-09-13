# Exploratory Release Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a verified macOS Mote build on `main` that restores cached folders immediately, indexes saved folders continuously, keeps the toolbar to one row, and copies picks safely with progress and cancellation.

**Architecture:** Run three isolated workstreams from the shared design commit: library continuity, toolbar/welcome presentation, and Picks/copy. Each workstream owns non-overlapping files where practical, commits test-first changes, and is merged sequentially into the integration branch before repository-wide verification and the final merge to `main`.

**Tech Stack:** Rust 2024 workspace, Tokio, SQLite-backed `photo-catalog`, Tauri 2, React 19, TypeScript 7, CSS Modules, Vitest Browser Mode, Biome, npm workspaces, macOS ad hoc signing.

**Spec:** `docs/superpowers/specs/2026-09-13-exploratory-release-fixes-design.md`

## Global Constraints

- Source media is read-only; never write, rename, or delete anything under a configured photo source.
- The welcome action uses the full configured accent at rest and hover and maintains at least 4.5:1 black-or-white text contrast.
- Desktop startup selects the naturally sorted top saved folder before access validation and never shows first-run UI when saved folders exist.
- Cached derivatives render before validation; confirmed folder failure flags the entire affected scope and skips per-file checks.
- Active-folder work has foreground priority; inactive saved-folder jobs continue with bounded lower concurrency and cannot starve.
- The top toolbar never wraps into a second row at supported widths.
- `Indexing - X of Y` is the only determinate indexing sentence; use `Indexing` when the total is unknown.
- A final destination filename is published only after its bytes are complete; no operation overwrites an existing destination file.
- Cancel retains completed files, removes only the current temporary file, keeps picks, restores the exact pre-attempt copy state, and reports `Copy cancelled` only in an at-most-one-second toast.
- The exact missing-destination message is `The destination folder no longer exists.`
- Do not add new production dependencies unless the integration owner approves them after reviewing the standard-library alternative.

## Worktree and ownership map

| Workstream | Branch | Owns |
| --- | --- | --- |
| Library continuity | `codex/release-library-continuity` | `crates/app-service` scan, bootstrap, saved-folder and gallery lifecycle; wall state/reducer/status files and their tests |
| Toolbar and welcome | `codex/release-toolbar-welcome` | `WallToolbar`, `SourceCanvas`, `PhotoTile`, `appShell.module.css`, `photoWall.module.css`, isolated browser presentation tests |
| Picks and copy | `codex/release-picks-copy` | copy service/desktop command state and DTOs; Picks controller/panel/toast/service contracts and tests; Picks documentation |
| Integration | `codex/exploratory-release-fixes` | sequential merges, conflict resolution, verification record, macOS build |

The workstream branches start from the integration branch after this plan is
committed. A worker must not edit another row's owned files without first
notifying the integration owner; that keeps the parallel merges reviewable.

---

## Workstream A — Library continuity

### Task 1: Ungated deterministic desktop bootstrap

**Files:**
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/app-service/src/saved_folders.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Test: `crates/app-service/tests/progressive_wall.rs`
- Test: `apps/desktop/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `AppService::bootstrap()`, `AppService::check_saved_folder()`, and the persisted `SavedFolderSnapshot`.
- Produces: `AppService::desktop_bootstrap() -> Result<BootstrapState, AppServiceError>`, which selects the naturally sorted first saved entry synchronously and schedules verification without awaiting it.

- [ ] **Step 1: Write failing startup tests**

Add tests that create saved entries named `Album 10`, `album 2`, and `Zoo`,
persist a stale or different active ID, block the folder probe, and assert that
bootstrap immediately returns `album 2` as both `active_entry_id` and
`active_source`. Rework the existing offline-reopen test to assert that cached
wall references remain queryable while the probe is blocked and after the
confirmed failure.

```rust
let bootstrap = timeout(Duration::from_millis(100), service.desktop_bootstrap())
    .await
    .expect("bootstrap must not await the access probe")?;
assert_eq!(bootstrap.saved_folders.active_entry_id.as_deref(), Some(album_2_id));
assert!(service.query_wall(WallQuery::first_page(100))?.items.iter()
    .all(|item| item.wall_thumbnail.is_some()));
```

- [ ] **Step 2: Run the focused tests and confirm the current gate fails**

Run: `cargo test -p photo-app-service --test progressive_wall desktop_bootstrap -- --nocapture`

Expected: FAIL because `checked_bootstrap` waits for validation or clears the
selection and cached wall.

- [ ] **Step 3: Implement deterministic selection and ungated bootstrap**

Add a Rust saved-folder ordering helper matching the interface contract:
case-insensitive natural numeric label, then display path, then stable ID. In
`desktop_bootstrap`, atomically set the first entry's group as the current
selection, return `bootstrap()` immediately, then spawn a generation-bound
verification task. Remove `checked_bootstrap()` from the Tauri
`get_bootstrap_state` path and from `AppService::open` startup reconciliation.

```rust
pub async fn desktop_bootstrap(&self) -> Result<BootstrapState, AppServiceError> {
    self.select_first_saved_folder()?;
    let bootstrap = self.bootstrap()?;
    self.spawn_restore_verification(bootstrap.saved_folders.active_entry_id.clone());
    Ok(bootstrap)
}
```

`Checking`, timeout, `Failed`, or invalid replies must not clear the selection.
Only a confirmed accessibility result may change availability.

- [ ] **Step 4: Run focused Rust and desktop tests**

Run: `cargo test -p photo-app-service --test progressive_wall desktop_bootstrap -- --nocapture`

Run: `cargo test -p photo-viewer-desktop commands::tests -- --nocapture`

Expected: PASS, including immediate cached bootstrap and topmost selection.

- [ ] **Step 5: Commit**

```bash
git add crates/app-service/src/service.rs crates/app-service/src/saved_folders.rs crates/app-service/tests/progressive_wall.rs apps/desktop/src-tauri/src/commands.rs
git commit -m "fix: restore the top saved folder from cache"
```

### Task 2: Make access validation root-first and scope-correct

**Files:**
- Modify: `crates/app-service/src/saved_folders.rs`
- Modify: `crates/app-service/src/gallery.rs`
- Modify: `crates/app-service/src/dto.rs`
- Modify: `crates/indexer/src/scanner.rs`
- Test: `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**
- Consumes: `FolderProbeOutcome::{Available, Missing, Unreadable, RootOffline}` and catalog `mark_root_offline`/`mark_group_offline`.
- Produces: generation-bound `WallUpdate::SourceUnavailable` or per-asset warning events after prerequisite folder checks.

- [ ] **Step 1: Write failing root/child probe tests**

Cover two saved child folders under one library. When the configured library
root is unreadable, assert that all library assets become `RootOffline` and no
child or asset probe runs. When the root is readable but the selected child is
missing, assert only that group's assets are flagged and no asset probe runs.
When both are readable, block the asset reader and prove cached query results
return before per-file work completes.

```rust
assert_eq!(probe_counts.root.load(Ordering::SeqCst), 1);
assert_eq!(probe_counts.child.load(Ordering::SeqCst), 0);
assert_eq!(probe_counts.asset.load(Ordering::SeqCst), 0);
assert!(catalog.assets_for_library(library)?.iter()
    .all(|asset| asset.availability == Availability::RootOffline));
```

- [ ] **Step 2: Run the new tests and observe scope/probe failures**

Run: `cargo test -p photo-app-service --test progressive_wall root_first -- --nocapture`

Expected: FAIL because a child selection currently converts `RootOffline` into
group-only state and the existing scan has no explicit per-asset availability
phase.

- [ ] **Step 3: Preserve probe outcomes and add asynchronous asset validation**

In `check_saved_folder`, branch on `FolderProbeOutcome::RootOffline` itself,
not whether the selected group is the library root. Use `mark_root_offline` for
that outcome, `mark_group_offline` for confirmed selected-child
missing/unreadable, and no durable downgrade for indeterminate replies. Admit
the indexer only after both prerequisite checks succeed. In `scanner.rs`, turn
an individual open/metadata failure into an asset-scoped unavailable result and
continue the walk; in `gallery.rs`, persist and publish that result without
turning one file failure into a scan-wide failure.

- [ ] **Step 4: Run access, catalog, and progressive wall tests**

Run: `cargo test -p photo-app-service --test progressive_wall root_first -- --nocapture`

Run: `cargo test -p photo-catalog`

Expected: PASS with the asserted zero downstream probes on prerequisite failure.

- [ ] **Step 5: Commit**

```bash
git add crates/app-service/src/saved_folders.rs crates/app-service/src/gallery.rs crates/app-service/src/dto.rs crates/app-service/tests/progressive_wall.rs crates/indexer/src/scanner.rs
git commit -m "fix: validate folder access before individual assets"
```

### Task 3: Keep one prioritized indexing job per saved folder

**Files:**
- Create: `crates/app-service/src/folder_jobs.rs`
- Modify: `crates/app-service/src/lib.rs`
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/app-service/src/scan.rs`
- Modify: `crates/app-service/src/derivative_coordinator.rs`
- Test: `crates/app-service/src/folder_jobs.rs`
- Test: `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**
- Consumes: stable `(LibraryId, FolderGroupId)` identity and `SelectionToken` only for view generations.
- Produces: `FolderJobRegistry::{ensure, set_foreground, finish, cancel_removed}` and `FolderJobPriority::{Foreground, Background}`.

- [ ] **Step 1: Write the scheduler tests**

Use controlled futures to start folder A, switch to B, and assert A remains
registered/running while B is serviced first. Add three background folders and
assert round-robin completion proves non-starvation. Repeated `ensure(A)` must
return the existing job rather than starting a second scan.

```rust
registry.ensure(folder_a, run_a.clone());
registry.set_foreground(folder_b);
registry.ensure(folder_b, run_b.clone());
assert_eq!(registry.running_count(folder_a), 1);
assert_eq!(next_started.recv().await, folder_b);
```

- [ ] **Step 2: Run scheduler tests and verify the single-active model fails**

Run: `cargo test -p photo-app-service folder_jobs -- --nocapture`

Expected: FAIL until the registry exists and selection changes no longer abort
the old scan.

- [ ] **Step 3: Implement the keyed priority registry**

Create `folder_jobs.rs` with a mutex-protected registry keyed by stable folder
identity. Give foreground work the existing responsive budget and background
work a smaller semaphore plus round-robin queue. Update selection transitions
to call `set_foreground` and leave the old job alive. Cancel only when its saved
folder is removed, its source is confirmed inaccessible, the service shuts
down, or that job reaches a terminal state.

Update derivative requests so visible active assets are promoted within their
folder while background folders retain a bounded lane. Reject stale UI events
by folder identity plus generation, not because another folder is active.

- [ ] **Step 4: Run scheduler and progressive integration tests**

Run: `cargo test -p photo-app-service folder_jobs -- --nocapture`

Run: `cargo test -p photo-app-service --test progressive_wall switching_folders -- --nocapture`

Expected: PASS; A continues in the background and B wins foreground scheduling.

- [ ] **Step 5: Commit**

```bash
git add crates/app-service/src/folder_jobs.rs crates/app-service/src/lib.rs crates/app-service/src/service.rs crates/app-service/src/scan.rs crates/app-service/src/derivative_coordinator.rs crates/app-service/tests/progressive_wall.rs
git commit -m "feat: continue indexing saved folders in the background"
```

### Task 4: Preserve loaded pages through metadata settlement

**Files:**
- Modify: `apps/interface/src/wall/wallReducer.ts`
- Modify: `apps/interface/src/app/usePhotoWall.ts`
- Test: `apps/interface/src/wall/wallReducer.test.ts`
- Test: `apps/interface/src/components/PhotoWall.browser.test.tsx`

**Interfaces:**
- Consumes: `metadataSettled` pages and scan/warning updates.
- Produces: monotonic `mergeSettledAssets(current, settled)`, plus indexing-first `wallProgress`.

- [ ] **Step 1: Add reducer and status regressions**

Create a state with 150 loaded items and a settlement response containing the
first 100. Assert all 150 remain, the first 100 receive settled metadata and
derivatives, and cursors/page exhaustion do not regress. Add a browser test that
introduces an item warning during an active scan and expects
`Indexing - 501 of 2056`, not `Some previews need attention`.

```ts
expect(settled.items).toHaveLength(150);
expect(settled.items.map(({ id }) => id)).toEqual(loadedIds);
expect(screen.getByRole("status")).toHaveTextContent("Indexing - 501 of 2056");
```

- [ ] **Step 2: Run the focused interface tests**

Run: `npm test --workspace @photo-viewer/interface -- src/wall/wallReducer.test.ts`

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/PhotoWall.browser.test.tsx`

Expected: FAIL because `metadataSettled` replaces the wall with its 100-item
response and warnings currently outrank scan progress.

- [ ] **Step 3: Merge settlement by ID and make indexing authoritative**

Change the reducer to merge the settled response into `state.items`, preserving
existing order and any loaded item absent from the response. Keep newer
derivative references and warning tombstones. In `wallProgress`, evaluate an
incomplete scan before warnings and format known totals exactly as
`Indexing - ${shaped} of ${total}`; use `Indexing` for an unknown total. Remove
all `Folder ready` and `Indexing photos` branches.

- [ ] **Step 4: Run reducer and browser suites**

Run: `npm test --workspace @photo-viewer/interface -- src/wall/wallReducer.test.ts`

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/PhotoWall.browser.test.tsx`

Expected: PASS, including the 150-item settlement and warning-during-indexing cases.

- [ ] **Step 5: Commit**

```bash
git add apps/interface/src/wall/wallReducer.ts apps/interface/src/wall/wallReducer.test.ts apps/interface/src/app/usePhotoWall.ts apps/interface/src/components/PhotoWall.browser.test.tsx
git commit -m "fix: preserve wall pages while indexing settles"
```

---

## Workstream B — Toolbar and welcome presentation

### Task 5: Keep adaptive contrast valid on the welcome action

**Files:**
- Modify: `apps/interface/src/components/SourceCanvas.tsx`
- Modify: `apps/interface/src/styles/appShell.module.css`
- Test: `apps/interface/src/components/App.browser.test.tsx`

**Interfaces:**
- Consumes: `--primary-control` and `--primary-control-text` set by `applyAccentColor`.
- Produces: welcome-only `welcomePrimaryButton` styling that never changes the accent fill.

- [ ] **Step 1: Add contrast tests for both foreground branches**

Render the welcome screen with `#777777`, verify computed black text has at
least 4.5:1 contrast at rest and hover, and assert hover keeps
`rgb(119, 119, 119)`. Add a dark-accent case that selects white. Confirm the
hosted `Open this folder` button does not receive the modifier.

- [ ] **Step 2: Run the contrast browser project**

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/App.browser.test.tsx`

Expected: FAIL on hover because the shared primary rule darkens `#777777` while
retaining black text.

- [ ] **Step 3: Add the local modifier and non-fill hover feedback**

```tsx
className={`${styles.primaryButton} ${styles.welcomePrimaryButton}`}
```

```css
.welcomePrimaryButton:not(:disabled):hover {
  background: var(--primary-control);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--focus-ring) 48%, transparent);
}
```

Do not change the shared token or shared `.primaryButton` hover rule.

- [ ] **Step 4: Run contrast tests**

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/App.browser.test.tsx`

Expected: PASS for rest, hover, black text, and white text.

- [ ] **Step 5: Commit**

```bash
git add apps/interface/src/components/SourceCanvas.tsx apps/interface/src/styles/appShell.module.css apps/interface/src/components/App.browser.test.tsx
git commit -m "fix: preserve welcome action contrast on hover"
```

### Task 6: Make desktop view controls icon-only and non-wrapping

**Files:**
- Modify: `apps/interface/src/components/WallToolbar.tsx`
- Modify: `apps/interface/src/styles/photoWall.module.css`
- Create: `apps/interface/src/components/WallToolbar.browser.test.tsx`

**Interfaces:**
- Consumes: existing `direction`, `galleryScope`, and change callbacks.
- Produces: accessible icon buttons with `aria-label`, `aria-pressed`, and CSS-only hover/focus tooltips.

- [ ] **Step 1: Write accessibility and geometry tests**

At desktop viewports, assert each control's visible content is only its SVG,
its accessible name remains `Oldest first`, `Newest first`, or
`Include subfolders`, and its tooltip appears on hover and focus. Measure all
toolbar children and assert they share one row at 900px, 1024px, and 1440px.

```ts
const tops = controls.map((control) => Math.round(control.getBoundingClientRect().top));
expect(new Set(tops).size).toBe(1);
expect(scope).toHaveAttribute("aria-pressed", "true");
```

- [ ] **Step 2: Run the browser test and see the wrapping/text failures**

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/WallToolbar.browser.test.tsx`

Expected: FAIL because labels are visible and `.sortControls` permits wrapping.

- [ ] **Step 3: Implement icon controls and one-row sizing**

Move labels to `aria-label`, add a reusable `data-tooltip` value, remove text
nodes, set `.sortControls { flex-wrap: nowrap; }`, and make view-options groups
non-shrinking while the concise status truncates first. Keep 44px accessible
hit targets even when the visual glyph is 16–18px.

- [ ] **Step 4: Run browser and accessibility coverage**

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/WallToolbar.browser.test.tsx`

Expected: PASS at all tested widths, with keyboard-focus tooltips and selected states.

- [ ] **Step 5: Commit**

```bash
git add apps/interface/src/components/WallToolbar.tsx apps/interface/src/styles/photoWall.module.css apps/interface/src/components/WallToolbar.browser.test.tsx
git commit -m "fix: keep photo controls on one toolbar row"
```

### Task 7: Hide add-to-picks until a preview is interactive

**Files:**
- Modify: `apps/interface/src/components/PhotoTile.tsx`
- Modify: `apps/interface/src/styles/photoWall.module.css`
- Create: `apps/interface/src/components/PhotoTile.browser.test.tsx`

**Interfaces:**
- Consumes: `TilePaintPhase` and `picked`.
- Produces: `canTogglePick = picked || phase === "interactive"`.

- [ ] **Step 1: Add lifecycle tests for the pick control**

Assert an unpicked tile has no add button while placeholder, decoding, or
fading; it appears after transition to interactive. Assert a picked tile keeps
`Remove … from picks` after a later source or derivative failure.

- [ ] **Step 2: Run the focused browser tests**

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/PhotoTile.browser.test.tsx`

Expected: FAIL because `PhotoTile` always renders the control.

- [ ] **Step 3: Gate only the add control**

Use `const canTogglePick = picked || phase === "interactive"` around the
separate pick button. Do not gate tile layout, the image transition, warning
badges, or removal of an existing pick.

- [ ] **Step 4: Run browser tests**

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/PhotoTile.browser.test.tsx`

Expected: PASS for all lifecycle phases and stale picked items.

- [ ] **Step 5: Commit**

```bash
git add apps/interface/src/components/PhotoTile.tsx apps/interface/src/styles/photoWall.module.css apps/interface/src/components/PhotoTile.browser.test.tsx
git commit -m "fix: show pick controls only for usable previews"
```

---

## Workstream C — Picks and desktop copy

### Task 8: Define distinct copy cancellation contracts

**Files:**
- Modify: `apps/interface/src/services/photoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.ts`
- Modify: `apps/interface/src/services/inMemoryPhotoService.ts`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Test: `apps/interface/src/services/tauriPhotoService.test.ts`

**Interfaces:**
- Produces: `CopyResult` variants `selectionCancelled`, `copyCancelled`, and `complete`; `PhotoService.cancelOriginalCopy(): Promise<void>`; native `cancel_original_copy` command.
- Consumes: operation IDs internal to the native layer; no filesystem paths cross IPC.

- [ ] **Step 1: Add adapter contract tests**

Assert picker dismissal returns `{kind: "selectionCancelled"}`, worker cancel
returns `{kind: "copyCancelled"}`, `cancelOriginalCopy` invokes
`cancel_original_copy`, and bounded native errors map the missing-destination
code to the approved sentence.

```ts
await service.cancelOriginalCopy();
expect(calls.at(-1)).toEqual(["cancel_original_copy", undefined]);
```

- [ ] **Step 2: Run the service tests**

Run: `npm test --workspace @photo-viewer/interface -- src/services/tauriPhotoService.test.ts`

Expected: FAIL until the distinct variants and cancel method exist.

- [ ] **Step 3: Implement the wire contract**

Replace the ambiguous `cancelled` copy result, add the PhotoService method to
all adapters, and register the Tauri command. Add
`copyDestinationMissing: "The destination folder no longer exists."` to the
bounded error map. Hosted/in-memory adapters return a resolved no-op for cancel.

- [ ] **Step 4: Run typecheck and adapter tests**

Run: `npm run typecheck`

Run: `npm test --workspace @photo-viewer/interface -- src/services/tauriPhotoService.test.ts`

Expected: PASS with exhaustive result handling.

- [ ] **Step 5: Commit**

```bash
git add apps/interface/src/services/photoService.ts apps/interface/src/services/tauriPhotoService.ts apps/interface/src/services/inMemoryPhotoService.ts apps/interface/src/services/tauriPhotoService.test.ts apps/desktop/src-tauri/src/dto.rs apps/desktop/src-tauri/src/lib.rs
git commit -m "feat: distinguish picker and copy cancellation"
```

### Task 9: Copy to private temporary files and publish without overwrite

**Files:**
- Modify: `crates/app-service/src/original_copy.rs`
- Test: `crates/app-service/src/original_copy.rs`

**Interfaces:**
- Produces: `CopyCancellation` backed by `Arc<AtomicBool>` and `copy_originals_with_control(batch, destination, cancellation, on_item)` returning `OriginalCopyOutcome::{Complete, Cancelled}`.
- Consumes: prepared immutable `OriginalCopyBatch` and existing destination containment checks.

- [ ] **Step 1: Write filesystem race and cancellation tests**

Add tests that cancel during a multi-megabyte first file and during a later
file, assert no `.mote-copy-*` files remain, completed final files are byte
correct, and remaining files never start. Poll the intended final name from a
reader and assert it is absent until it contains the complete payload. Create a
competing final name immediately before publication and assert neither file is
overwritten. Delete the destination tree during a copy and expect the distinct
destination-missing outcome.

- [ ] **Step 2: Run the original-copy tests**

Run: `cargo test -p photo-app-service original_copy -- --nocapture`

Expected: FAIL because copying currently writes directly to the final name and
has no cancellation token.

- [ ] **Step 3: Implement chunked copy and no-replace publication**

Open a unique destination-local `.mote-copy-{operation}-{item}` file with
`create_new(true)`. Replace `io::copy` with a bounded buffer loop that checks
`CopyCancellation::is_cancelled()` between reads/writes and again before
publication. After `sync_all` and close, publish without overwrite using
same-directory `hard_link(temp, final)` followed by temp unlink; on
`AlreadyExists`, reserve another collision suffix and retry publication.

```rust
pub enum OriginalCopyOutcome {
    Complete(OriginalCopyResult),
    Cancelled,
}

while !cancel.is_cancelled() {
    let read = input.read(&mut buffer).map_err(|_| "copy_failed")?;
    if read == 0 { break; }
    output.write_all(&buffer[..read]).map_err(|_| "copy_failed")?;
}
```

Revalidate the destination before each file and before publication. Map a
missing directory to `CopyDestinationMissing`, clean only the current temp file
by verified identity, and never iterate over completed final outputs. Do not
remember a destination for a cancelled outcome.

- [ ] **Step 4: Run copy and source-safety tests**

Run: `cargo test -p photo-app-service original_copy -- --nocapture`

Run: `cargo test -p photo-app-service source -- --nocapture`

Expected: PASS with no partial final names, no overwrites, and no temp debris.

- [ ] **Step 5: Commit**

```bash
git add crates/app-service/src/original_copy.rs
git commit -m "fix: publish copied originals atomically"
```

### Task 10: Open the destination picker before preparing the batch

**Files:**
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Test: `apps/desktop/src-tauri/src/state.rs`
- Test: `apps/desktop/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: Task 9 `CopyCancellation` and `OriginalCopyOutcome`.
- Produces: `CopyOperationRegistry::{begin, cancel, finish}` with monotonically increasing operation ID and cancellation handle.

- [ ] **Step 1: Add command ordering and registry race tests**

Record calls and assert the picker callback runs before `list_photo_picks` or
`prepare_original_copy`. Test cancel during first/later items, double cancel,
cancel waiting for cleanup, and a late cancel that cannot affect the next
operation ID. Deleting the destination must stop the remaining batch and return
the bounded missing-destination error/result after cleaning its temp file.

- [ ] **Step 2: Run desktop command tests**

Run: `cargo test -p photo-viewer-desktop commands::tests -- --nocapture`

Expected: FAIL because batch preparation currently precedes the picker and the
only operation state is an exclusive mutex.

- [ ] **Step 3: Implement operation state and immediate picker flow**

Keep the single-copy guard, but add a separate registry entry containing the
operation ID and cancellation token so `cancel_original_copy` never waits on
the held guard. In `run_original_copy`, load only the remembered destination,
open the picker, return `SelectionCancelled` silently if dismissed, then prepare
the immutable IDs/batches. Await the blocking worker and cleanup before
returning `CopyCancelled`; clear the registry only when that exact ID finishes.

- [ ] **Step 4: Run desktop command and DTO tests**

Run: `cargo test -p photo-viewer-desktop commands::tests -- --nocapture`

Expected: PASS for picker order, cancellation generations, cleanup, and missing destination.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/state.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src-tauri/src/dto.rs
git commit -m "feat: cancel desktop original copies safely"
```

### Task 11: Present resettable toasts and drawer-owned copy state

**Files:**
- Modify: `apps/interface/src/picks/usePickList.ts`
- Modify: `apps/interface/src/components/PickToast.tsx`
- Modify: `apps/interface/src/components/PicksPanel.tsx`
- Create: `apps/interface/src/styles/pickToast.module.css`
- Modify: `apps/interface/src/styles/picksPanel.module.css`
- Test: `apps/interface/src/picks/usePickList.test.tsx`
- Test: `apps/interface/src/components/PicksPanel.browser.test.tsx`

**Interfaces:**
- Consumes: Task 8 `PhotoService.cancelOriginalCopy` and distinct copy results.
- Produces: `PickListController.cancelCopy()`, `PickCopyState.phase: "cancelling"`, and `PickToastState.phase: "visible" | "exiting"`.

- [ ] **Step 1: Add controller timer/state tests**

With fake timers, add a pick, advance into the final fade, add another pick,
and assert a new toast ID returns to `visible` and disappears no later than one
second after the second add. Test reduced motion, silent picker dismissal,
explicit cancel waiting for the native promise, exact restoration of both idle
and prior partial states, toast-only `Copy cancelled`, and Clear resetting copy
messages/failures to the initial state.

- [ ] **Step 2: Add panel geometry and action tests**

During copying, assert the toolbar Picks trigger contains no progress element,
the drawer has one full-width progress row with right-aligned `2 of 6`, and
Cancel replaces Clear. During cancellation, assert `Cancelling...` is disabled.
After completion then Clear, assert `Copied 6 originals`, Show folder, and row
errors are absent.

- [ ] **Step 3: Run focused Picks tests and confirm failures**

Run: `npm test --workspace @photo-viewer/interface -- src/picks/usePickList.test.tsx`

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/PicksPanel.browser.test.tsx`

Expected: FAIL on toast timing/fade, missing cancel action, toolbar progress, and
stale completion state.

- [ ] **Step 4: Implement controller transitions**

Publish `Added to picks` immediately with the optimistic add instead of waiting
for persistence. Use a short timer whose whole visible-plus-exiting lifetime is
at most 1,000ms; keep Clear Undo's existing five-second timer independent.
Snapshot `previous` before copy. `selectionCancelled` restores without a toast;
`copyCancelled` restores only after the cancel command and worker resolve, then
publishes the short toast. `clear()` resets `copy` to the initial state after
the persisted clear succeeds.

- [ ] **Step 5: Implement drawer presentation**

Remove progress rendering from the `PicksTrigger` function in `PicksPanel.tsx`.
Add a `.copyProgress` grid with
`grid-template-columns: minmax(0, 1fr) auto`, set its `<progress>` to
`width: 100%`, and render `X of N`. Replace Clear with Cancel during `copying`
and disabled `Cancelling...` during `cancelling`. Keep closing the panel from
cancelling the operation.

- [ ] **Step 6: Run Picks unit/browser tests and typecheck**

Run: `npm test --workspace @photo-viewer/interface -- src/picks/usePickList.test.tsx`

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/PicksPanel.browser.test.tsx`

Run: `npm run typecheck`

Expected: PASS with copy feedback contained in the drawer and transient feedback in toasts.

- [ ] **Step 7: Commit**

```bash
git add apps/interface/src/picks/usePickList.ts apps/interface/src/picks/usePickList.test.tsx apps/interface/src/components/PickToast.tsx apps/interface/src/components/PicksPanel.tsx apps/interface/src/components/PicksPanel.browser.test.tsx apps/interface/src/styles/pickToast.module.css apps/interface/src/styles/picksPanel.module.css
git commit -m "fix: make Picks copy progress cancellable and contained"
```

### Task 12: Update user-facing copy documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-12-persistent-photo-picks-design.md`

**Interfaces:**
- Consumes: the implemented behavior from Tasks 8–11.
- Produces: documentation with no obsolete Clear-during-copy or toolbar-progress claims.

- [ ] **Step 1: Update the prior Picks contract**

State that the picker opens first; copy progress appears only in the drawer;
Cancel replaces Clear; cancellation retains completed files, removes the current
temp file, restores the previous drawer state, and emits only the short toast.
Document destination disappearance and finalized-file publication.

- [ ] **Step 2: Scan for contradictory copy wording**

Run: `rg -n "toolbar|Clear|Cancel|copy|destination|partial" README.md docs/superpowers/specs/2026-09-12-persistent-photo-picks-design.md`

Expected: every match agrees with the new design; no text permits clearing
picks during a running copy or direct writes to final filenames.

- [ ] **Step 3: Commit**

```bash
git add README.md docs/superpowers/specs/2026-09-12-persistent-photo-picks-design.md
git commit -m "docs: describe cancellable atomic original copies"
```

---

## Integration

### Task 13: Review and merge each workstream

**Files:**
- Modify as needed: only merge-conflict sites from the workstream ownership map

**Interfaces:**
- Consumes: completed, reviewed workstream branch heads.
- Produces: one integrated `codex/exploratory-release-fixes` history.

- [ ] **Step 1: Run each branch's focused verification before merge**

Capture the exact command, exit code, and commit SHA for every workstream. Do
not accept a branch with dirty files, skipped new tests, or an unexplained
warning/error.

- [ ] **Step 2: Perform requirements and code-quality review**

Review each diff against its numbered spec outcomes. Confirm tests fail on the
old behavior and pass on the branch, public types match the Interfaces blocks,
and no source write path or unrelated UI change was introduced.

- [ ] **Step 3: Merge in dependency order**

```bash
git merge --no-ff codex/release-library-continuity
git merge --no-ff codex/release-toolbar-welcome
git merge --no-ff codex/release-picks-copy
```

Resolve conflicts semantically and rerun the focused tests for every file that
required resolution.

- [ ] **Step 4: Verify the integrated diff is clean**

Run: `git diff --check main...HEAD`

Run: `git status --short`

Expected: no whitespace errors and no uncommitted files.

### Task 14: Run the release-candidate verification matrix

**Files:**
- Create: `docs/superpowers/verification/2026-09-13-exploratory-release-fixes.md`

**Interfaces:**
- Consumes: integrated source tree.
- Produces: evidence record containing commands, exit codes, key counts, macOS artifact identity, and manual smoke observations.

- [ ] **Step 1: Run formatting, type, and interface suites**

Run: `npm run check`

Run: `npm run typecheck`

Run: `npm test`

Run: `npm run test:browser`

Expected: all commands exit 0.

- [ ] **Step 2: Run Rust workspace verification**

Run: `npm run rust:verify`

Expected: formatting, Clippy, and all Rust tests exit 0 with no warnings treated as errors.

- [ ] **Step 3: Run packaging and hosted regression tests**

Run: `npm run brand:test`

Run: `npm run test:deployment`

Run: `npm run test:flatpak`

Run: `npm run web:build`

Expected: all commands exit 0.

- [ ] **Step 4: Build and inspect the native macOS app**

Run: `npm run desktop:build`

Run: `codesign --verify --deep --strict dist/macos/Mote.app`

Run: `file dist/macos/Mote.app/Contents/MacOS/Mote`

Expected: fresh ad hoc-signed arm64 app passes strict verification.

- [ ] **Step 5: Perform the manual smoke test**

Launch the built app with two previously indexed folders. Verify top-folder
restore, immediate cached previews, background indexing after switching,
single-row controls, scroll continuity, pick gating/toasts, normal copy,
reader-safe final files, Cancel cleanup, and destination deletion. Record each
check and inspect the destination for `.mote-copy-*` debris.

- [ ] **Step 6: Write and commit the verification record**

```bash
git add docs/superpowers/verification/2026-09-13-exploratory-release-fixes.md
git commit -m "docs: record exploratory release fix verification"
```

### Task 15: Merge the verified release fixes into main

**Files:**
- No source edits expected

**Interfaces:**
- Consumes: clean, verified integration branch and macOS artifact.
- Produces: `main` containing all fixes and the verification record.

- [ ] **Step 1: Confirm both worktrees and branch heads**

Run: `git status --short --branch`

Run: `git log --oneline --decorate -8`

Expected: integration is clean and contains all three reviewed merge commits.

- [ ] **Step 2: Merge using the user's already approved integration choice**

From the primary `main` worktree:

```bash
git merge --no-ff codex/exploratory-release-fixes
```

- [ ] **Step 3: Re-run the final fast verification on main**

Run: `npm run check`

Run: `npm run typecheck`

Run: `git status --short --branch`

Expected: checks pass; only the pre-existing ignored/untracked release artifacts
explicitly preserved by the integration owner may remain outside Git.
