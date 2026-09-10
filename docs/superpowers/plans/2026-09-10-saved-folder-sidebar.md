# Saved folder sidebar implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Execute inline unless the user chooses delegation.

**Goal:** Implement the approved saved-folder sidebar on desktop and hosted web, with editable labels, independent browser persistence, and coordinated availability checks.

**Architecture:** Keep shortcuts separate from catalog libraries. Extend the shared PhotoService contract, persist desktop entries in SQLite and web entries in browser storage, and coordinate folder access in the Rust backend. Preserve existing gallery runtimes and cached viewing while enforcing current selection intent.

**Tech stack:** Rust, SQLite/rusqlite, Tokio, Axum, Tauri, React, TypeScript, TanStack Query, Vitest and the existing Playwright browser suites.

**Spec:** [Saved folder sidebar design](../specs/2026-09-10-saved-folder-sidebar-design.md).

**Approved UX:** [Option 3, Roomy rows](../specs/assets/2026-09-10-saved-folder-sidebar-approved.png). Read the spec's Approved visual UX section for exact states and responsive behaviour.

## Global constraints

- Source media remains read-only.
- Renaming changes a shortcut label, and removal deletes a shortcut.
- No user accounts or server-side personal folder lists are needed.
- Hosted paths remain root-relative; never return absolute server paths.
- The cooldown applies to success and failure and cannot be bypassed by a client refresh flag.
- Reuse results completed less than five seconds ago, and allow a caller to stop waiting after five seconds.
- A timed-out filesystem probe retains its in-progress slot until the operation ends.
- Keep the current wall visible if its folder becomes unavailable; returning after leaving requires successful access.
- Keep existing source-containment and overlapping-source rules.
- Desktop sidebar: 288px wide at widths of 900px and above, 52px rows, 15px labels.
- Below 900px, use the labelled navigation drawer with targets at least 44px.
- Warning badges: top-right, inset 8px, 24px size, normal image colour and opacity.
- Use existing Mote assets, theme tokens, gallery layout, controls and dependencies.
- Preserve the existing toolchain floors: Rust 1.97.1, Node.js 24.18.0 and npm 11.16.0.
- Do not modify unrelated local changes. At planning time CI, README, package.json, .gitignore and packaging outputs already had unrelated changes.

## Execution preparation and file boundaries

At execution time inspect git status, read applicable repository instructions,
and use the worktree skill if isolation is required. No implementation or
tests were run while authoring this plan. The code blocks below specify new
contracts and representative regression cases; implement them in the named
files and extend the existing fixtures for the listed behaviours.

| Boundary | Files to create | Existing integration points |
| --- | --- | --- |
| Shared UI model | `apps/interface/src/folders/savedFolders.ts`, `selectionIntent.ts` | `services/photoService.ts`, all three adapters |
| Desktop persistence | `crates/catalog/src/saved_folder_repo.rs`, migration `0012_saved_folders.sql` | `catalog/src/lib.rs`, `migrate.rs` |
| Access coordination | `crates/app-service/src/folder_access.rs` | `app-service/src/lib.rs`, `service.rs`, `gallery.rs` |
| Desktop commands | `crates/app-service/src/saved_folders.rs` | desktop `commands.rs`, `dto.rs`, `lib.rs`, `state.rs` |
| Hosted access API | `crates/server/src/api/folder_access.rs` | server `folders.rs`, `api/gallery.rs`, `api/types.rs`, `api/mod.rs`, `lib.rs` |
| Browser list store | `apps/interface/src/services/browserSavedFolders.ts` | `browserPreferences.ts`, `httpPhotoService.ts` |
| Sidebar UX | `components/SavedFolderList.tsx`, `SavedFolderRow.tsx`, `styles/savedFolders.module.css` | `NavigationRail.tsx`, `AppShell.tsx`, `useAppController.ts` |
| Source warnings | `components/SourceUnavailableBadge.tsx` | `PhotoTile.tsx`, `PhotoWallCanvas.tsx`, `JustifiedWall.tsx`, `PhotoViewerOverlay.tsx`, `ViewerStage.tsx`, `WallToolbar.tsx` |

Keep new behaviour in these focused modules. Do not expand the already large
`gallery.rs`, `service.rs`, `httpPhotoService.ts`, or `AppShell.tsx` with the
whole feature. Extract only the selection/check helpers this work needs.

### Shared TypeScript contract

Add these definitions in `folders/savedFolders.ts`; import the types into
`photoService.ts`. `folderId` is an opaque stable catalog folder-group UUID,
not a selection epoch or display label. `id` is a shortcut ID; removing and
re-adding produces a new shortcut ID while retaining `folderId`.

```ts
export type FolderAccessState =
  | "unknown" | "checking" | "available"
  | "missing" | "unreadable" | "rootOffline" | "unverified";

export interface SavedFolder {
  id: string;
  folderId: string;
  name: string;
  displayPath: string;
  customLabel: string | null;
}

export interface FolderAccess {
  folderId: string;
  state: FolderAccessState;
  generation: number;
  retryAfterMs: number;
}

export interface SavedFolderSnapshot {
  entries: SavedFolder[];
  access: Record<string, FolderAccess>;
  activeEntryId: string | null;
  hasOpenedFolder: boolean;
  persistenceError: string | null;
}

export function folderLabel(entry: SavedFolder): string;
export function normalizeFolderLabel(value: string): string | null;
export function sortSavedFolders(entries: readonly SavedFolder[]): SavedFolder[];
```

Extend `PhotoService` with the following methods, rather than optional
capability-specific methods. All adapters and test doubles must implement
them. Bootstrap adds `savedFolders: SavedFolderSnapshot`. Keep the existing
selection result union and source summary fields for existing consumers.

```ts
getSavedFolders(): SavedFolderSnapshot;
watchSavedFolders(listener: (snapshot: SavedFolderSnapshot) => void): () => void;
renameSavedFolder(id: string, label: string): Promise<BootstrapState>;
removeSavedFolder(id: string): Promise<BootstrapState>;
activateSavedFolder(id: string): Promise<ChooseFolderResult>;
clearActiveFolder(): Promise<BootstrapState>;
checkSavedFolders(ids: readonly string[]): Promise<SavedFolderSnapshot>;
```

The service adapter owns mutation order and emits snapshots. The React
controller subscribes and invalidates view intents; it does not mutate raw
browser storage or assemble native filesystem paths. `getBootstrapState`
restores the active entry only after the access gate. Startup checks of other
entries continue without delaying the entire interface.

## Task 1: Define folder labels, stable snapshots, and selection intent

**Files:** Create `apps/interface/src/folders/savedFolders.ts`,
`savedFolders.test.ts`, `selectionIntent.ts`, `selectionIntent.test.ts`.
Modify `services/photoService.ts`, `inMemoryPhotoService.ts`,
`tauriPhotoService.ts`, `httpPhotoService.ts`, their existing tests, and the
service fixtures in component tests.

**Consumes:** Existing `BootstrapState`, `ChooseFolderResult`, service adapters.
**Produces:** The shared contract above, and
`createSelectionIntent(): { begin(): number; invalidate(): void; isCurrent(token: number): boolean }`.

- [ ] Add pure-model regression tests before implementing the helpers:

```ts
import { expect, it } from "vitest";
import { normalizeFolderLabel, sortSavedFolders, type SavedFolder } from "./savedFolders";

it("sorts displayed labels naturally without changing caller order", () => {
  const entries: SavedFolder[] = [
    { id: "a", folderId: "ga", name: "Z", displayPath: "/Z", customLabel: "album 10" },
    { id: "b", folderId: "gb", name: "Album 2", displayPath: "/B", customLabel: null },
  ];
  expect(sortSavedFolders(entries).map(entry => entry.id)).toEqual(["b", "a"]);
  expect(entries.map(entry => entry.id)).toEqual(["a", "b"]);
  expect(normalizeFolderLabel("  ")).toBeNull();
});
```

- [ ] Run `npm run --workspace @photo-viewer/interface test -- src/folders/savedFolders.test.ts` and confirm failure for the missing helpers.
- [ ] Implement trimming, immutable sorting, and deterministic path/ID ties.
  Use `Intl.Collator(undefined, { numeric: true, sensitivity: "base" })` for
  labels, followed by code-unit path/ID comparison for ties. Keep an empty
  custom label as null, never overwrite the stored folder name.
- [ ] Implement and test the selection-intent counter:

```ts
export function createSelectionIntent() {
  let current = 0;
  return {
    begin: () => ++current,
    invalidate: () => { current += 1; },
    isCurrent: (token: number) => token === current,
  };
}
```

- [ ] Add the contract to `PhotoService`. Implement the in-memory adapter's
  complete add/deduplicate/rename/remove/activate flow and emit cloned
  snapshots. During this contract commit, real adapters expose empty lists
  and fail unsupported new mutations with a typed error; the UI does not use
  these paths until Tasks 4-7 replace them. Do not silently report success.
- [ ] Update all structural service test doubles and bootstrap fixtures;
  run `npm run typecheck` and the interface unit tests. Commit as
  `feat: define saved folder state and selection intent`.

## Task 2: Persist desktop shortcuts independently of catalog roots

**Files:** Create `crates/catalog/migrations/0012_saved_folders.sql`,
`crates/catalog/src/saved_folder_repo.rs`,
`crates/catalog/tests/saved_folders.rs`. Modify `crates/catalog/src/lib.rs`,
`migrate.rs`, and the migration unit tests in `migrate.rs`.

**Consumes:** `Catalog`, `FolderGroupId`, `StoredSourceSelection` and existing
`folder_groups`/`active_source_selection` tables.
**Produces:** `SavedFolderRecord { id: uuid::Uuid, folder_group_id: FolderGroupId, custom_label: Option<String> }`
and these catalog methods:

```rust
pub fn list_saved_folders(&self) -> Result<Vec<SavedFolderRecord>, CatalogError>;
pub fn save_folder(&mut self, group: FolderGroupId) -> Result<SavedFolderRecord, CatalogError>;
pub fn save_and_activate_folder(&mut self, group: FolderGroupId) -> Result<SavedFolderRecord, CatalogError>;
pub fn rename_saved_folder(&mut self, id: uuid::Uuid, label: Option<&str>) -> Result<(), CatalogError>;
pub fn remove_saved_folder(&mut self, id: uuid::Uuid) -> Result<bool, CatalogError>;
```

`remove_saved_folder` returns whether the removed entry was active. A missing
ID is an idempotent no-op returning false. Rename of a missing ID is an error.

- [ ] Add an integration test using a temporary database: create a library
  and folder group with the existing `NewLibrary::recent` and
  `NewFolderGroup` fixtures, save twice, and assert equal IDs and one row.
  Rename, reopen the database, and assert the custom label survives. Extend
  it to remove the active shortcut and assert an empty active selection,
  while `find_library` and `folder_group` still return their records.

```rust
#[test]
fn saving_a_group_twice_preserves_its_label() {
    use photo_catalog::{Catalog, NewFolderGroup, NewLibrary};
    use photo_domain::{FolderGroupId, RelativePathKey};
    use std::path::Path;
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog.add_library(&NewLibrary::recent("Photos", Path::new("/Photos"))).unwrap();
    let group = catalog.upsert_folder_group(&NewFolderGroup {
        id: FolderGroupId::new(), library_id: library.id,
        relative_path: RelativePathKey::from_relative_path(Path::new("")).unwrap(),
        display_path: "Photos".into(), last_viewed_at: None,
    }).unwrap();
    let first = catalog.save_folder(group).unwrap();
    catalog.rename_saved_folder(first.id, Some("Family")).unwrap();
    let second = catalog.save_folder(group).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(second.custom_label.as_deref(), Some("Family"));
    assert_eq!(catalog.list_saved_folders().unwrap().len(), 1);
}
```
- [ ] Run `cargo test -p photo-catalog --test saved_folders`; expect missing
  repository APIs. Add the migration and register it after migration 11:

```sql
CREATE TABLE saved_folders (
    id BLOB PRIMARY KEY NOT NULL CHECK(length(id) = 16),
    folder_group_id BLOB NOT NULL UNIQUE REFERENCES folder_groups(id),
    custom_label TEXT
);
CREATE TABLE saved_folder_preferences (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    has_opened_folder INTEGER NOT NULL CHECK(has_opened_folder IN (0, 1))
);
INSERT INTO saved_folder_preferences VALUES (1, 0);
PRAGMA user_version = 12;
```

- [ ] In the version-12 migration transaction, seed only the previously
  active group, matching library ID and native relative-folder key. Generate
  a UUID in Rust. If the active selection has no group yet, upsert that group
  from its catalog identity without touching the source. Set the history bit
  when seeding. Migration must not run again after removal.
- [ ] `save_folder` returns the existing row on a group conflict without
  altering its label, and sets the history bit. Removal checks active
  library/relative identity, clears `active_source_selection`, and deletes
  the shortcut in one transaction. No library/group/asset/cache deletions.
  Export `has_opened_folder(&self) -> Result<bool, CatalogError>` for bootstrap.
  `save_and_activate_folder` performs the same upsert plus active-selection
  replacement using the group's library and relative key in one transaction;
  share connection-level helpers to avoid nested rusqlite transactions.
- [ ] Test duplicate labels on different groups, null-label reset, unrelated
  profile databases, corrupt IDs, migration with/without an active selection,
  and source bytes/mtime unchanged. Run `cargo test -p photo-catalog` and
  commit as `feat: persist desktop saved folder shortcuts`.

## Task 3: Coordinate access probes with a shared cooldown

**Files:** Create `crates/app-service/src/folder_access.rs`. Modify
`crates/app-service/src/lib.rs`. Put controlled-clock and controlled-probe
unit tests in the new module.

**Consumes:** Trusted catalog `LibraryId`, `RelativePathKey` and native root
paths; `tokio::time::Instant`, `spawn_blocking`, and bounded synchronization.
**Produces:** A cloneable `FolderAccessCoordinator`, `FolderAccessKey`,
`FolderAccessTarget`, `FolderProbe`, `FolderProbeOutcome`, `AccessReply`, and
`ValidatedFolder`. These are Rust-only types; native paths are never serialized.

```rust
pub struct FolderAccessKey {
    pub library_id: photo_domain::LibraryId,
    pub relative: photo_domain::RelativePathKey,
}
pub struct FolderAccessTarget {
    pub key: FolderAccessKey,
    pub root: std::path::PathBuf,
}
pub trait FolderProbe: Send + Sync + 'static {
    fn probe(&self, target: &FolderAccessTarget) -> FolderProbeOutcome;
}
// All types used in replies are Clone; keys also implement Eq and Hash.
pub enum AccessReply {
    Complete { generation: u64, outcome: FolderProbeOutcome, retry_after_ms: u64 },
    Checking { generation: u64 },
}
pub enum FolderProbeOutcome {
    Available(ValidatedFolder), Missing, Unreadable, RootOffline, Invalid, Failed,
}
// Constructor is private to the probe module. Stores key and canonical path.
pub struct ValidatedFolder {
    key: FolderAccessKey,
    canonical_path: std::path::PathBuf,
}
```

- [ ] Write a test probe backed by an atomic invocation counter and a
  condition variable. Its `probe` increments the counter, waits for the test
  to release it, then returns Missing. Construct a coordinator with it,
  start two `check(target.clone())` calls, release the gate, and assert
  both return the same generation and the counter equals one.

  Also include a fast deterministic shared-result test:

```rust
#[tokio::test]
async fn simultaneous_callers_reuse_one_missing_result() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    struct CountProbe(Arc<AtomicUsize>);
    impl FolderProbe for CountProbe {
        fn probe(&self, _: &FolderAccessTarget) -> FolderProbeOutcome {
            self.0.fetch_add(1, Ordering::SeqCst);
            FolderProbeOutcome::Missing
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let coordinator = FolderAccessCoordinator::new(Arc::new(CountProbe(calls.clone())));
    let target = FolderAccessTarget {
        key: FolderAccessKey {
            library_id: photo_domain::LibraryId::new(),
            relative: photo_domain::RelativePathKey::from_relative_path(
                std::path::Path::new("Family")).unwrap(),
        },
        root: std::path::PathBuf::from("/unused-by-test-probe"),
    };
    let (first, second) = tokio::join!(
        coordinator.check(target.clone()), coordinator.check(target));
    assert!(matches!(first, AccessReply::Complete { outcome: FolderProbeOutcome::Missing, .. }));
    assert!(matches!(second, AccessReply::Complete { outcome: FolderProbeOutcome::Missing, .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
```
- [ ] Run `cargo test -p photo-app-service folder_access`; expect the missing
  coordinator. Implement `FolderAccessCoordinator::new(Arc<dyn FolderProbe>)`
  and `async fn check(&self, target: FolderAccessTarget) -> AccessReply`.
  `ValidatedFolder` privately stores `key: FolderAccessKey` and
  `canonical_path: PathBuf`; expose read-only getters for service consumers.
- [ ] Implement admission as a short mutex-protected map lookup: join a
  running slot, return a fresh completed result, or install a running slot
  with a new generation before spawning work. Use four global probe permits
  and at most 64 waiting callers per key. Excess callers receive Checking.
  Never hold the map mutex or catalog lock while waiting or probing.
- [ ] Use `tokio::time::Instant` for a five-second completed-result TTL and
  a five-second caller deadline. The worker owns completion and releases
  its slot only after `probe` really returns. Catch worker panic and publish
  a retryable internal failure without leaving the slot occupied forever.
  Use `FolderProbeOutcome::Failed` for that outcome; adapters map it to
  unverified, never to a confirmed missing folder.
- [ ] Use a `watch` channel per running slot so callers can share completion
  without individual result queues. Retain completed slots for the cooldown;
  prune expired idle slots. Bound queued/running distinct keys to 256;
  return Checking on overflow without creating a slot. Background checking
  clients retry overflow only on their next normal trigger.
- [ ] The production probe canonicalizes the trusted root-relative target,
  verifies containment, verifies directory type, and opens `read_dir` without
  traversing children. Classify a child error separately from root failure;
  a root check uses the same coordination mechanism. Share the root prerequisite
  check when several children need it, rather than independently probing an
  unavailable network root for every child. Do not start scans or derivatives.
- [ ] Add paused-clock tests for success/failure TTL boundaries, repeat
  clicks, different folder keys, timeout with a still-running worker, panic
  cleanup, bounded callers and keys, generation ordering, and an expired
  result becoming available. Confirm the invocation count remains one after
  a timeout until the old worker exits. Commit as
  `feat: coordinate folder availability checks across clients`.

## Task 4: Activate and manage saved folders through the desktop backend

**Files:** Create `crates/app-service/src/saved_folders.rs` and
`crates/app-service/tests/saved_folders.rs`. Modify app-service `service.rs`,
`scan.rs`, `dto.rs`, `lib.rs`; desktop `commands.rs`, `lib.rs`, `state.rs`,
`dto.rs`; and `apps/interface/src/services/tauriPhotoService.ts` plus its tests.

**Consumes:** Catalog shortcut methods, access coordinator, existing
selection request sequence, selection epoch, protected groups and scan bridge.
**Produces:** App-service DTOs matching the TypeScript saved snapshot and:

```rust
// Methods on AppService; all errors use existing AppServiceError plus typed
// saved-folder missing, unavailable and check-in-progress variants.
pub async fn restore_saved_folder(&self) -> Result<BootstrapState, AppServiceError>;
pub fn saved_folder_snapshot(&self) -> Result<SavedFolderSnapshot, AppServiceError>;
pub async fn activate_saved_folder(&self, id: uuid::Uuid) -> Result<BootstrapState, AppServiceError>;
pub fn rename_saved_folder(&self, id: uuid::Uuid, label: &str) -> Result<BootstrapState, AppServiceError>;
pub fn remove_saved_folder(&self, id: uuid::Uuid) -> Result<BootstrapState, AppServiceError>;
pub fn clear_active_folder(&self) -> Result<BootstrapState, AppServiceError>;
pub async fn check_saved_folders(&self, ids: Vec<uuid::Uuid>) -> Result<SavedFolderSnapshot, AppServiceError>;
```

- [ ] Add a regression case to the new integration file using the existing
  `AppConfig` temporary-directory pattern:

```rust
#[tokio::test]
async fn removing_active_shortcut_preserves_source_and_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Family");
    std::fs::create_dir(&source).unwrap();
    let config = photo_app_service::AppConfig::new(
        temp.path().join("data"), temp.path().join("cache"));
    let service = photo_app_service::AppService::open(config).unwrap();
    service.open_recent(&source).unwrap();
    let snapshot = service.saved_folder_snapshot().unwrap();
    assert_eq!(snapshot.entries.len(), 1);
    let id = uuid::Uuid::parse_str(&snapshot.entries[0].id).unwrap();
    let state = service.remove_saved_folder(id).unwrap();
    assert!(state.active_source.is_none());
    assert!(source.is_dir());
    assert!(service.saved_folder_snapshot().unwrap().entries.is_empty());
}
```

- [ ] Run `cargo test -p photo-app-service --test saved_folders`; confirm the
  missing service behaviour. Add serde DTOs with camelCase wire fields.
  Resolve names and full display paths from catalog native paths, never from
  a client-supplied desktop path string. Preserve group identity for children.
- [ ] Add the short saved-selection commit helper
  `commit_existing_saved_folder(&self, id: uuid::Uuid, validated: ValidatedFolder, request_id: u64) -> Result<(BootstrapState, SelectionToken), AppServiceError>`.
  Obtain trusted native paths and the request token before asynchronous access
  checks; commit only if the request is current and the shortcut still exists.
  New picker selections still use `select_recent`, the existing SourceValidator
  and overlap rules; use `save_and_activate_folder` for that successful
  selection transaction. Existing saved selections commit their known library/group
  identity directly after checking the proof, instead of constructing the
  core crate's private ValidatedSourceFolder or repeating filesystem validation.
  Factor the common epoch/protection/bridge update into a helper shared with
  `select_recent`; do not duplicate the old source-selection lifecycle.
- [ ] In `remove_saved_folder` and `clear_active_folder`, supersede pending
  activation requests before mutating storage. Clearing/removing active bumps
  the selection epoch, clears scan ownership, releases the old protected
  group, and detaches the old desktop update bridge. Keep the underlying
  gallery/catalog/cache reusable. Failed access never replaces the current
  active source. Rename checks current access state and persists only an
  available entry's label; return the custom label as the visible source title.
- [ ] Move automatic desktop restoration/scan launch behind
  `restore_saved_folder`; keep synchronous bootstrap suitable for appearance
  setup but do not expose an unverified restored folder as active. Call the
  async restoration from the existing get-bootstrap command. Startup checks
  for remaining rows are separate from the restoration dependency.
- [ ] Register commands `get_saved_folders`, `activate_saved_folder`,
  `rename_saved_folder`, `remove_saved_folder`, `clear_active_folder`, and
  `check_saved_folders`. Run native blocking picker and probe work away from
  the async executor; retain the existing native portal picker boundary.
  Extend error mapping with user-safe unavailable/checking/missing messages.
- [ ] Replace the Tauri adapter's temporary unsupported methods. Update its
  snapshot after command results and emit change notifications. During an
  access check emit checking locally and apply only the latest generation.
  A timed-out pending activation must not be retried as an automatic open.
- [ ] Test reopen/profile isolation, duplicate entry and custom-label title,
  migration of previous selection, unavailable restoration, removal during a
  controlled delayed check, and stale desktop events after clearing. Update
  old bootstrap tests that intentionally asserted path omission: absolute
  paths are now permitted only in desktop saved-entry DTOs, never hosted DTOs.
  Run app-service tests and Tauri adapter tests; run desktop cargo check with
  `--manifest-path apps/desktop/src-tauri/Cargo.toml`. Commit as
  `feat: manage saved folders through desktop services`.

## Task 5: Expose coordinated hosted access and stable root identity

**Files:** Create `crates/server/src/api/folder_access.rs` and
`crates/server/tests/folder_access_api.rs`. Modify server `lib.rs`, `folders.rs`,
`api/mod.rs`, `api/types.rs`, `api/gallery.rs`; app-service `gallery.rs`;
and existing `folder_api.rs`, `gallery_api.rs`, `source_disabled.rs` tests.

**Consumes:** One `Arc<FolderAccessCoordinator>` owned by AppState, the
configured library identity and contained-root validation.
**Produces:** An opaque `rootId` on bootstrap, a stable `folderId` and
canonical root-relative `path` on selection summaries, and this endpoint:

```http
POST /api/v1/folder-access
Content-Type: application/json

{"folderIds":["8de97f96-c3f1-4cd5-86ed-3482dfe1fd85"]}
```

```json
{
  "rootId": "library-opaque-id",
  "results": [{
    "folderId": "8de97f96-c3f1-4cd5-86ed-3482dfe1fd85",
    "state": "available",
    "generation": 1,
    "retryAfterMs": 5000
  }]
}
```

Accept at most 64 unique folder IDs in a batch. Reject malformed IDs, unknown
body fields, duplicate IDs and oversized bodies with the current invalid
request envelope. Unknown IDs produce a missing entry result without leaking
another root's catalog data. Checking is a normal per-entry result in HTTP
200; it must not claim confirmed unavailability.

- [ ] Extend the existing router fixture to issue concurrent folder-access
  requests against the same registered group using the controlled probe.
  Assert one invocation and identical generations. Add these wire assertions:

```rust
assert_eq!(first["results"][0]["folderId"], second["results"][0]["folderId"]);
assert_eq!(first["results"][0]["generation"], second["results"][0]["generation"]);
assert!(first["rootId"].is_string());
assert!(!first.to_string().contains(source.to_str().unwrap()));
assert_eq!(probe_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
```

- [ ] Run `cargo test -p photo-server --test folder_access_api`; expect the
  missing endpoint. Add route/DTO parsing and map access outcomes to the
  exact shared access-state names. Root identity uses the configured catalog
  library UUID and survives process restart; no raw paths or content hashes.
- [ ] Replace bootstrap's direct `root.is_available()` request-thread I/O
  with a coordinated root check and last-known status. A null configured
  root reports `rootId: null`, an empty list namespace, and unavailable
  capability. Do not erase a browser's previous list on transient startup
  errors or interpret missing root identity as a new root.
- [ ] Refactor hosted selection into a probe phase outside catalog locks and
  `GalleryEngine::select_validated(&self, folder: ValidatedFolder)` for the
  short catalog commit. Preserve `select_relative` as a compatibility wrapper
  over the same safe validation path for other callers/tests. Remove the
  current duplicate `root.resolve` plus `select_relative` canonicalization
  from create-selection. Return folder-group UUID and canonical relative path
  alongside the existing selection ID, name and breadcrumbs.
- [ ] New paths have no group yet: use the configured root plus syntactically
  normalized relative key for check admission. After successful canonical
  resolution, register the canonical key for future saved-folder checks.
  Two requests for the same normalized path share the initial probe.
  Existing saved IDs map to canonical group keys without filesystem I/O.
  Keep containment checks inside the probe, and normal file-serving safety
  checks intact; a recent access result is not a grant to read arbitrary paths.
- [ ] Classify missing/unreadable children separately from offline root;
  only confirmed root failure updates library-wide offline state. Share root
  checks across concurrent children. If a request is cancelled or times out,
  retain the server probe and do not launch a new one from a selection retry.
- [ ] Test sibling independence, success/failure cooldown, root restart
  identity, known offline root, traversal/symlink escape, batch bounds, and
  two client selections remaining independent. Run `cargo test -p photo-server`
  and the app-service hosted selection/runtime tests. Commit as
  `feat: add shared hosted folder access checks`.

## Task 6: Persist web shortcuts and synchronize lists across tabs

**Files:** Create `apps/interface/src/services/browserSavedFolders.ts` and
`browserSavedFolders.test.ts`. Modify `browserPreferences.ts`,
`browserPreferences.test.ts`, `httpPhotoService.ts`, `httpPhotoService.test.ts`.

**Consumes:** Shared saved-folder model and new hosted root/folder identity
and access DTOs. Uses injected `Storage`, storage-event subscription, UUID
factory and selection-intent counter for deterministic tests.
**Produces:** `createBrowserSavedFolders(options: BrowserSavedFolderOptions): BrowserSavedFolderStore`:

```ts
export interface BrowserSavedFolderOptions {
  rootId: string;
  localStorage: Storage;
  sessionStorage: Storage;
  randomUuid(): string;
  subscribeStorage(listener: (key: string | null) => void): () => void;
}
export interface BrowserSavedFolderStore {
  read(): SavedFolderSnapshot;
  add(folder: Omit<SavedFolder, "id" | "customLabel">): SavedFolder;
  rename(id: string, label: string): void;
  remove(id: string): void;
  setActive(id: string | null): void;
  setAccess(result: FolderAccess): void;
  subscribe(listener: () => void): () => void;
  dispose(): void;
}
```

- [ ] Add a two-instance storage regression using the existing MemoryStorage
  fixture, with explicit storage-event delivery. Give each instance separate
  sessionStorage and shared localStorage. Assert this sequence:

```ts
const family = first.add({ folderId: "family", name: "Family", displayPath: "Family" });
const coast = second.add({ folderId: "coast", name: "Coast", displayPath: "Trips/Coast" });
first.rename(family.id, "Family favourites");
second.setActive(coast.id);
expect(second.read().activeEntryId).toBe(coast.id);
expect(first.read().entries).toHaveLength(2);
expect(first.add({ folderId: "family", name: "Family", displayPath: "Family" }).id).toBe(family.id);
expect(first.read().entries.find(entry => entry.id === family.id)?.customLabel).toBe("Family favourites");
```

- [ ] Run `npm run --workspace @photo-viewer/interface test -- src/services/browserSavedFolders.test.ts`; expect the missing store. Use per-entry
  localStorage keys of `photo-viewer.saved-folders.v1:<rootId>:entry:<folderId>`.
  The entry key enforces deduplication even if tabs add the same folder at
  once. Re-read committed storage after writing and deliver fresh snapshots.
  Use last-writer-wins for a same-entry collision; separate entries never
  overwrite an entire list. Clear events cause a snapshot refresh too.
- [ ] Store last-opened and history metadata under separate root-scoped keys.
  Session keys store `{ activeEntryId: string | null }`; distinguish no key
  from explicit null. On refresh prefer the session key, and on a new tab
  fall back to last-opened. Removing a shared active entry invalidates that
  tab's intent, clears its local active selection and wall resources, and
  records explicit null. A storage event for another tab's last-opened value
  must never navigate an already initialized tab.
- [ ] Validate all stored fields. Catch Storage access and write exceptions;
  maintain the current in-memory snapshot and set persistenceError to
  `Folder changes cannot be saved in this browser.` Suppress storage events
  from other roots and stale lower access generations. Do not persist access
  results as durable truth across page loads.
- [ ] Migrate legacy selectionId/breadcrumbs once after bootstrap establishes
  the root and selection summary yields folderId/path. Preserve viewing
  preferences and leave an unavailable/unresolved legacy selection pending
  rather than fabricating a group or losing it. Mark migration complete only
  after a successful durable write or a confirmed nonexistent legacy selection.
  Update strict HTTP decoders for new fields without loosening unknown-field,
  enum or bounds validation. Root changes switch namespaces and invalidate
  pending requests before applying results.
- [ ] Implement all HTTP saved-folder methods and bootstrap restoration.
  Activation uses the shared access endpoint and opens only after available;
  posting a selection then reuses the same backend result. Preserve cached
  content for an already active unavailable folder. Use service intent tokens
  before changing `activeSource`, preferences, subscriptions, or returning a
  selected result; stale requests return cancelled. Requests for the current
  active folder do not unnecessarily tear down its resources.
  Both HTTP and Tauri adapters return cancelled for known-unavailable,
  checking, or superseded activation after publishing the relevant snapshot;
  reserve thrown errors for unexpected request/storage failures. Snapshot
  access state supplies the warning instead of a duplicate blocking dialog.
- [ ] Batch automatic access checks in groups of 64 with at most four
  outstanding requests. Expose checking state and return remaining unknown
  states promptly so a large list never blocks rendering. A Checking response
  after the caller deadline ends automatic opening; subsequent explicit
  activation can check again. Do not add continuous polling.
- [ ] Test two-browser isolation, two-tab shared edits, same-folder add race,
  storage failure, malformed input, root replacement, refresh versus new tab,
  pending selection removal, and late completions. Run interface unit tests
  and typecheck. Commit as `feat: save hosted folder shortcuts per browser`.

## Task 7: Implement the approved sidebar and responsive drawer

**Files:** Create `apps/interface/src/components/SavedFolderList.tsx`,
`SavedFolderRow.tsx`, `SavedFolderList.browser.test.tsx`, and
`styles/savedFolders.module.css`. Modify `NavigationRail.tsx`, `AppShell.tsx`,
`SourceCanvas.tsx`, `app/useAppController.ts`, `styles/appShell.module.css`,
and `components/App.browser.test.tsx`.

**Consumes:** PhotoService saved-folder methods, shared label/sort helpers,
snapshot subscription, current drawer focus handling, approved UX image.
**Produces:** An accessible `SavedFolderList` with the props below; the
existing NavigationRail remains the brand/navigation container.

```ts
export interface SavedFolderListProps {
  snapshot: SavedFolderSnapshot;
  onActivate(id: string): void;
  onRename(id: string, label: string): Promise<void>;
  onRemove(id: string): Promise<void>;
  onAdd(): void;
}
```

- [ ] Add a browser test with a saved Family entry, an unavailable Archive
  entry, and spies for the callback props. Render the list inside the existing
  themed test wrapper. The test should verify the unavailable row can request
  a recheck while its menu excludes Rename:

```tsx
await screen.getByRole("button", { name: "Check Archive availability" }).click();
expect(onActivate).toHaveBeenCalledWith("archive-entry");
await screen.getByRole("button", { name: "Options for Archive" }).click();
await expect.element(screen.getByRole("menuitem", { name: "Remove" })).toBeVisible();
expect(screen.getByRole("menuitem", { name: "Rename" }).query()).toBeNull();
```

- [ ] Run `npm run --workspace @photo-viewer/interface test:browser -- src/components/SavedFolderList.browser.test.tsx`; expect missing UI. Build
  a semantic nav list with sibling activation and menu buttons. Available
  activation labels are `Open <label>`; unavailable labels are
  `Check <label> availability`; checking labels are `Checking <label>`.
  Use `aria-current` for active selection, accessible tooltip descriptions,
  and a polite shared status announcement. Do not use native disabled on
  the recheck activation control. The backend remains the cooldown authority.
- [ ] Apply the approved layout constants and existing brand assets:

```css
.list { min-height: 0; overflow-y: auto; }
.row { display: flex; align-items: center; min-height: 52px; }
.activate { display: flex; flex: 1; min-width: 0; align-items: center; }
.label { min-width: 0; overflow: hidden; text-overflow: ellipsis;
  white-space: nowrap; font-size: 15px; }
.menuTrigger { flex: 0 0 44px; width: 44px; height: 44px; }
/* In the existing shell stylesheet, replace the old rail breakpoints. */
@media (min-width: 900px) {
  .appShell { grid-template-columns: 288px minmax(0, 1fr); }
}
```

- [ ] Keep the Mote wordmark at 92px, sidebar horizontal padding at 24px,
  sentence-case Folders and the plus/Add folder control on one line, and
  header outside the scrolling list. Retain visible ellipses on every row,
  6px row radii, subtle green active stripe and existing focus token. Use the
  matching Lucide folder, ellipsis, triangle-alert and progress icons.
- [ ] Implement a viewport-contained menu with Rename/Remove, Escape and
  outside-click dismissal, and focus restoration. Use an inline input with
  current label selected, Enter to save, Escape/blur to cancel, and no sorting
  until save. On successful save re-sort and return focus to that entry;
  failed persistence preserves the editor with a useful error. On removal
  focus next, previous, then Add folder. Respect an entry becoming unavailable
  while its editor/menu is open by cancelling rename and retaining removal.
- [ ] Wire snapshot subscription through `useAppController` and feed the
  same list into the rail and drawer. Focus/startup checks use the same service
  path, coalesce focus events, and clean up listeners on unmount. Cache data
  with TanStack Query through the existing bootstrap key; service snapshots
  remain the source of truth. Update selection only for current controller
  intent and current adapter result. Remove-active closes the viewer and
  clears wall state, selection, and return anchors before another render can
  restore a stale tile.
- [ ] Replace the 640-899px icon-only rail with the existing drawer flow;
  below 900px hide the permanent rail and show its menu trigger. Drawer width
  is `min(320px, calc(100% - 48px))`; retain focus trapping, Escape/backdrop
  behaviour, 52px rows, minimum 44px targets and inert background. Keep the
  drawer open on failed activation and close only when it succeeds. Opening
  the folder picker from this drawer retains the existing nested focus rules.
- [ ] Add the post-removal empty state using snapshot.hasOpenedFolder and
  no active entry: title `Select a folder`, body
  `Choose a saved folder or open a new one.`, action `Add folder`.
  The original first-run welcome remains when hasOpenedFolder is false.
- [ ] Cover Enter/Escape/blur rename, sorting with focus retention, same-label
  paths, active/inactive removal, check cooldown, keyboard path tooltips,
  pending activation after removal, and drawer behaviour at 390px, 768px and
  desktop widths. Run the relevant browser suites and typecheck. Commit as
  `feat: add the approved saved folder sidebar`.

## Task 8: Preserve cached viewing and add source-unavailable warnings

**Files:** Create `components/SourceUnavailableBadge.tsx` and
`SourceUnavailableBadge.browser.test.tsx`. Modify `PhotoTile.tsx`,
`JustifiedWall.tsx`, `PhotoWallCanvas.tsx`, `WallToolbar.tsx`,
`PhotoViewerOverlay.tsx`, `ViewerStage.tsx`, `ViewerFilmstrip.tsx`,
`app/usePhotoWall.ts`, `wall/wallReducer.ts`, `styles/photoWall.module.css`,
`styles/photoViewer.module.css`, `styles/tokens.css`,
`PhotoWall.browser.test.tsx`, `PhotoViewer.browser.test.tsx` and reducer tests.

**Consumes:** Active saved entry's access state, existing per-asset
availability/warnings, decoded derivative state, wall and viewer sequence.
**Produces:** A reusable warning badge and source-status presentation.
`SourceUnavailableBadge` accepts `{ description: string }`; it is visual and
non-interactive. The owning tile/viewer exposes the same description accessibly.

- [ ] Extend the existing browser wall fixture: open a cached photo, emit
  sourceUnavailable for its current selection, and assert that the wall and
  viewer retain their images. Then switch folders and try returning while
  access still fails; the current selection must not change. Use assertions:

```tsx
await expect.element(screen.getByRole("img", { name: "Coast" })).toBeVisible();
await expect.element(screen.getByText("Source unavailable. Showing cached images.")).toBeVisible();
expect(service.getSavedFolders().activeEntryId).toBe("family-entry");
await service.activateSavedFolder("coast-entry");
const retry = await service.activateSavedFolder("family-entry");
expect(retry.kind).toBe("cancelled");
expect(service.getSavedFolders().activeEntryId).toBe("coast-entry");
```

- [ ] Run the focused wall/viewer browser cases and confirm the new source
  warning assertions fail. Keep source access separate from derivative
  decode/readiness; do not set a successfully displayed thumbnail back to a
  loading phase merely because its source became unavailable.
- [ ] Add a source warning token using amber `#f3c45b` on dark surfaces and
  a darker accessible amber on light surfaces. Badge backgrounds stay
  translucent graphite over images in both themes. Implement the placement:

```css
.sourceUnavailableBadge {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 24px;
  height: 24px;
  display: grid;
  place-items: center;
  border-radius: 6px;
  background: rgb(16 18 22 / 78%);
  color: #f3c45b;
  pointer-events: none;
}
```

- [ ] Add the badge to affected cached tiles and a source warning near the
  immersive viewer controls. Provide a tooltip on the owning tile's hover
  and keyboard focus, with description
  `Source unavailable. Showing cached image.` Preserve the open overlay's
  click target, filmstrip sequence, zoom and panning. Keep generic per-image
  warnings separate in meaning and avoid stacking duplicate corner badges;
  the description may state both source and preview problems when necessary.
- [ ] Use the existing wall status area for
  `Source unavailable. Showing cached images.` Preserve already loaded
  items on an availability event or temporary query failure. Requests that
  fail for a previously active source cannot reset the list to empty. Items
  with no usable cached derivative use the existing unavailable fallback and
  cannot open. A browser cannot fetch an uncached derivative while the server
  is unreachable; keep decoded content and show a fallback for failed loads.
- [ ] Bind sourceUnavailable events to the active entry snapshot. A folder
  recovery result removes only the source warning; retain individual errors.
  Switching away ends the cached-view exception. Starting/refreshing the app
  still uses the saved-entry access gate. Never start source-file reads to
  recover an unavailable cached preview.
- [ ] Test partial cache, no cache, root offline versus one corrupt file,
  recovery, no badge click interception, keyboard descriptions, and stale
  old-selection events. Run existing viewer motion/contrast and wall suites
  to catch overlay/focus regressions. Commit as
  `feat: show source warnings while retaining cached photos`.

## Task 9: Verify the complete desktop and hosted journeys

**Files:** Extend `tests/hosted/hosted.spec.ts`,
`crates/app-service/tests/task7_source_safety.rs` and
`crates/app-service/tests/saved_folders.rs`. Update `README.md` and
`docs/deployment/hosted.md` only in the relevant folder/persistence sections.
Create `docs/superpowers/verification/2026-09-10-saved-folder-sidebar.md` and
place verification captures under its existing `assets/` directory.

**Consumes:** Completed Tasks 1-8 and the approved mockup.
**Produces:** Verification evidence tied to the final implementation.

- [ ] Extend the hosted end-to-end test with two browser contexts and two
  tabs in one context. Verify Add, default label, Rename, natural order,
  refresh persistence, cross-tab list edits, independent active selections,
  and cross-context isolation. For a tab removal use:

```ts
await tabA.getByRole("button", { name: "Options for Family" }).click();
await tabA.getByRole("menuitem", { name: "Remove" }).click();
await expect(tabA.getByRole("heading", { name: "Select a folder" })).toBeVisible();
await expect(tabB.getByRole("button", { name: "Open Family", exact: true })).toHaveCount(0);
await expect(otherBrowser.getByRole("button", { name: "Open Family", exact: true })).toBeVisible();
```

- [ ] Exercise shared unavailable-folder clicks with the server test probe
  count, then restore access and verify only the still-current click intent
  opens the folder. Verify that removal while checking cannot recreate the
  shortcut, and that a late result from a prior root is discarded.
- [ ] Run the source-safety fixture through save, rename, active and inactive
  removal, re-add, source outage and cached viewing. Compare original source
  bytes and mtimes before/after. Assert catalog and derivative records survive
  shortcut removal and are still subject to normal cache eviction.
- [ ] Run the complete required checks once the focused tests pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
npm run check
npm run typecheck
npm test
npm run test:browser
npm run --workspace @photo-viewer/interface build
npm run web:build
npm run test:hosted
```

The desktop crate is excluded from the root Rust workspace; its separate
checks are necessary. The hosted suite's current setup/fixture requirements
are in `tests/hosted/playwright.config.ts`; use that fixture rather than an
unrelated live server. Do not run source-state changes against real photos.

- [ ] Preview the final app in the in-app browser and capture desktop dark
  and light at 1440x1024, tablet at 768px, and phone at 390px. Check available,
  active unavailable, menu, rename, checking, long-list and cleared-wall
  states. Compare sidebar and warning styling with approved option 3; retain
  the real justified photo arrangement. Check full labels via tooltips,
  non-clipped menus, correct focus and no photo-covering warning banners.
- [ ] Record which checks actually ran, their outcomes, any platform limits,
  and capture paths. Update docs for browser-persistent lists, desktop profile
  storage, shared five-second checks and cached viewing. Commit as
  `test: verify saved folder journeys across hosts`.

## Completion review

- [ ] Every agreed interaction and every state in the approved UX has a
  corresponding implemented task and behavioural check.
- [ ] No temporary unsupported adapter methods from Task 1 remain.
- [ ] All wire fields, enum names, method signatures and test doubles agree.
- [ ] Hosted saved lists and labels never enter shared catalog app_state.
- [ ] An expired caller does not release a running filesystem probe's slot.
- [ ] A removed entry or superseded selection cannot be restored by late work.
- [ ] Only the approved mockup and final implementation evidence are described
  as selected/verified; generated alternatives are not implementation output.
- [ ] Inspect the final diff for unrelated changes before the execution
  workflow handles branch integration.

## Spec coverage map

| Requirement | Tasks |
| --- | --- |
| Flat list, add/reuse, labels, natural sorting | 1, 2, 4, 6, 7 |
| Rename and remove without modifying source/catalog/cache | 2, 4, 7, 9 |
| Desktop persistence and migration | 2, 4 |
| Browser persistence, root identity, tabs and migration | 5, 6, 9 |
| Shared checks, cooldown, timeouts and slow shares | 3, 4, 5, 6, 9 |
| No stale response reopening a removed or previous folder | 1, 4, 6, 7, 9 |
| Offline active viewing and warnings | 4, 6, 8, 9 |
| Paths, containment and source safety | 2, 3, 4, 5, 9 |
| Option 3 layout, menus, keyboard and drawer | 7, 8, 9 |
| Empty state, restoration and first-run distinction | 2, 4, 6, 7 |
