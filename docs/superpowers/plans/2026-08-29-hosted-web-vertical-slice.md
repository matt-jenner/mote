# Hosted web vertical slice implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the existing Photo Viewer experience as one responsive OCI container in which each browser independently selects a folder beneath a read-only `/photos` mount.

**Architecture:** Keep React behind the host-neutral `PhotoService` boundary and add an HTTP adapter for hosted browsers. Refactor the Rust application layer around selection-explicit gallery operations and a shared runtime registry, while the existing Tauri methods remain desktop-state wrappers. `photo-server` owns contained folder browsing, JSON and SSE transport, managed derivative delivery, static hosting, and health.

**Tech Stack:** Rust 1.97.1, Axum 0.8.9, Tokio 1.53.1, SQLite through rusqlite 0.40.2, React 19.2.8, TypeScript 7.0.2, Vite 8.2.2, Vitest 4.1.11, Playwright 1.62.1, standard OCI `Containerfile`, Podman for local acceptance.

**Spec:** `docs/superpowers/specs/2026-08-29-hosted-web-vertical-slice-design.md`

## Global constraints

- Source media is read-only. Production code may read beneath `PHOTO_VIEWER_SOURCE_ROOT` but may not create, update, rename, or delete source files.
- The container source root is exactly `/photos`; data and cache remain separate writable roots at `/var/lib/photo-viewer` and `/var/cache/photo-viewer`.
- Every browser owns its selected folder, appearance, gallery scope, and sort direction. Do not persist hosted browser state in SQLite `app_state`, a cookie, or a server-global active selection.
- A tab owns a random client instance ID in `sessionStorage`. The ID is scheduling state, not identity or authentication.
- React components import only the host-neutral `PhotoService`; they do not import Tauri or HTTP modules.
- Desktop behavior and Tauri commands remain compatible. Hosted selection-explicit methods must not silently mutate the desktop active selection.
- Scans are recursive. `GalleryScope` affects SQLite queries and derivative demand, never SMB traversal.
- Wall pages accept 1 to 250 rows and use keyset cursors bound to selection ID, direction, and scope.
- HTTP request targets accept at most 8 KiB of query text and 8 decoded query parameters. Singleton parameters may not repeat. Folder paths accept at most 4096 bytes, cursors 2048 bytes, opaque selection IDs 128 ASCII bytes, opaque derivative IDs 512 bytes after decoding, client IDs 128 ASCII bytes, and event IDs 20 ASCII decimal bytes.
- Hosted derivative responses resolve an opaque identifier through SQLite and managed cache containment. They never fall back to source media.
- SSE queues are bounded. Lag produces `resyncRequired`, not unbounded buffering.
- All static, API, event, and derivative URLs are origin-relative. Do not add `PHOTO_VIEWER_PUBLIC_URL` or trust forwarded hosts for security decisions.
- The server binds to `PHOTO_VIEWER_BIND`, defaults only where the approved spec permits, and emits no absolute redirect.
- The first hosted slice has no app authentication, PWA service worker, video playback, remote administration, sharing, or general file-manager operations.
- Keep video assets invisible in wall and viewer responses.
- Respect the current responsive, touch-target, keyboard, focus, contrast, reduced-motion, viewer, zoom, and pan behavior.
- Work on a feature branch created from `codex/include-subfolders-gallery`. Do not merge or push before user acceptance.

## File map

- `crates/catalog/migrations/0009_selection_membership.sql` adds many-to-many folder-group membership so simultaneous overlapping selections cannot reassign assets.
- `crates/catalog/src/selection_repo.rs` owns stable group lookup, asset membership, group reconciliation, and derivative membership queries.
- `crates/catalog/src/wall_repo.rs` keeps keyset wall queries but joins explicit selection membership.
- `crates/indexer/src/catalog_writer.rs` records the selected group on discovery without overwriting another group’s membership.
- `crates/app-service/src/gallery.rs` defines stable `GallerySelection`, selection-explicit wall, scan, derivative, and cache-read operations.
- `crates/app-service/src/hosted_runtime.rs` owns the shared selection runtime registry, bounded events, client interaction leases, and demand aggregation.
- `crates/app-service/src/service.rs`, `scan.rs`, and `derivatives.rs` retain desktop methods as wrappers over the selection-explicit layer.
- `crates/server/src/folders.rs` validates mount-relative folder paths and performs one-level directory listing.
- `crates/server/src/api/` contains path-free HTTP DTOs, error mapping, gallery handlers, SSE, and derivative delivery.
- `crates/server/src/static_host.rs` serves the Vite bundle with interface-route fallback and security/cache headers.
- `apps/interface/src/services/httpPhotoService.ts` maps HTTP/SSE to `PhotoService`.
- `apps/interface/src/services/browserPreferences.ts` owns versioned local and session storage.
- `apps/interface/src/wall/wallReducer.ts` preserves the browser-owned sort direction across selection and resync resets.
- `apps/interface/src/components/HostedFolderBrowser.tsx` is the responsive contained folder dialog/sheet.
- `Containerfile`, `deploy/compose.yaml`, `deploy/nginx.conf.example`, and `docs/deployment/hosted.md` define the production contract.
- `tests/hosted/hosted.spec.ts` and `scripts/hosted-smoke.sh` verify two-browser independence, restart, offline cache, traversal rejection, and source safety.

---

### Task 1: Contained source root and lazy folder API

**Demo checkpoint:** With a temporary source tree, `curl /api/v1/bootstrap` reports availability and `curl /api/v1/folders?path=Trips` returns only immediate child directories without leaking the native root.

**Files:**
- Modify: `crates/server/Cargo.toml`
- Modify: `crates/server/src/config.rs`
- Modify: `crates/server/src/lib.rs`
- Modify: `crates/server/src/main.rs`
- Create: `crates/server/src/api/mod.rs`
- Create: `crates/server/src/api/error.rs`
- Create: `crates/server/src/api/types.rs`
- Create: `crates/server/src/folders.rs`
- Create: `crates/server/tests/folder_api.rs`
- Modify: `crates/server/tests/health_api.rs`

**Interfaces:**
- Consumes: `ServerConfig::{data_dir, cache_dir, catalog_path, bind}` and the existing `AppState`/`build_router` health boundary.
- Produces:

```rust
pub struct ServerConfig {
    local: LocalStatePaths,
    bind: SocketAddr,
    source_root: PathBuf,
    web_root: PathBuf,
}

impl ServerConfig {
    pub fn new(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        bind: Option<&str>,
        source_root: PathBuf,
        web_root: PathBuf,
    ) -> Result<Self, ConfigError>;
    pub fn source_root(&self) -> &Path;
    pub fn web_root(&self) -> &Path;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub path: String,
    pub breadcrumbs: Vec<FolderBreadcrumb>,
    pub children: Vec<FolderEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderBreadcrumb { pub name: String, pub path: String }

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry { pub name: String, pub path: String }

pub struct ContainedFolderRoot { root: PathBuf }

#[derive(Debug, thiserror::Error)]
pub enum FolderError {
    #[error("invalid relative folder path")]
    InvalidPath,
    #[error("folder resolves outside the source root")]
    OutsideRoot,
    #[error("folder is unavailable")]
    Unavailable,
    #[error("path is not a directory")]
    NotDirectory,
    #[error("folder is unreadable")]
    Unreadable,
}

impl ContainedFolderRoot {
    pub fn new(root: PathBuf) -> Result<Self, FolderError>;
    pub fn list(&self, relative: &str) -> Result<FolderListing, FolderError>;
    pub fn resolve(&self, relative: &str) -> Result<PathBuf, FolderError>;
}
```

- [ ] **Step 1: Write failing configuration and containment tests**

Add tests that construct `ServerConfig` with a real source directory and assert all of these cases:

```rust
assert_eq!(config.source_root(), source.canonicalize().unwrap());
assert!(matches!(root.list("../outside"), Err(FolderError::InvalidPath)));
assert!(matches!(root.list("/absolute"), Err(FolderError::InvalidPath)));
assert!(matches!(root.list("photo.jpg"), Err(FolderError::NotDirectory)));
assert!(!serde_json::to_string(&root.list("Trips").unwrap()).unwrap().contains(source.to_str().unwrap()));
```

On Unix, create one symlink to a child inside the root and one to an outside directory. Assert that the in-root link lists successfully after canonical validation and the escaping link returns `FolderError::OutsideRoot`.

At the HTTP layer, assert a decoded `path` of 4097 bytes and a repeated `path` parameter return the fixed `invalidFolderPath` response before `read_dir` runs.

- [ ] **Step 2: Run the focused tests and confirm RED**

Run: `cargo test -p photo-server --test folder_api --test health_api`

Expected: compilation fails because `ContainedFolderRoot`, `FolderError`, and the new `ServerConfig::new` parameters do not exist.

- [ ] **Step 3: Implement configuration validation and one-level listing**

Read `PHOTO_VIEWER_SOURCE_ROOT` as required, `PHOTO_VIEWER_DATA_DIR` and `PHOTO_VIEWER_CACHE_DIR` as required, `PHOTO_VIEWER_BIND` as optional with the current default, and `PHOTO_VIEWER_WEB_ROOT` as optional with `/app/web` as its default. Canonicalize the existing source root before local-state preparation and reject a missing root, non-directory root, and any canonical or symlink-alias overlap with data/cache.

In `ContainedFolderRoot::resolve`, accept the empty string as the mounted root. Reject NUL bytes, absolute paths, `.` and `..`, and empty interior components such as `Trips//Processed`. Canonicalize the joined result, require a readable directory, and verify `canonical.starts_with(&self.root)`. In `list`, call exactly one `std::fs::read_dir`, keep directories only, validate each child through the same containment check, omit symlinks that escape the root, sort case-insensitively by display name with the original name as a tie-breaker, and return mount-relative slash-separated paths. A direct request for an escaping symlink returns `invalidFolderPath`.

Reject a decoded folder `path` over 4096 bytes or a repeated `path` parameter before calling `ContainedFolderRoot`. Task 4 adds the shared 8 KiB request-target and 8-parameter guard when the full API router exists.

Map failures to fixed responses:

```rust
ApiError::new(StatusCode::BAD_REQUEST, "invalidFolderPath", "That folder path is not valid.")
ApiError::new(StatusCode::NOT_FOUND, "folderUnavailable", "That folder is unavailable.")
ApiError::new(StatusCode::FORBIDDEN, "folderUnreadable", "That folder cannot be read.")
ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "sourceUnavailable", "The photo source is unavailable.")
```

Add `/api/v1/bootstrap` and `/api/v1/folders` routes. `bootstrap` returns `{ "capabilities": { "folderBrowser": true, "video": false }, "sourceAvailable": bool }` and no filesystem path.

- [ ] **Step 4: Verify folder API behavior and source immutability**

Run: `cargo test -p photo-server --test folder_api --test health_api`

Expected: PASS, including a before/after snapshot of source bytes, lengths, and modified times around bootstrap and folder listing.

- [ ] **Step 5: Commit the feature checkpoint**

```bash
git add crates/server
git commit -m "feat: browse hosted photo folders safely"
```

---

### Task 2: Stable selections and overlapping catalogue membership

**Demo checkpoint:** Two stable selection IDs can represent a parent and child folder at the same time; both return correct cached wall rows after either selection is queried or rescanned.

**Files:**
- Create: `crates/catalog/migrations/0009_selection_membership.sql`
- Create: `crates/catalog/src/selection_repo.rs`
- Modify: `crates/catalog/src/lib.rs`
- Modify: `crates/catalog/src/cache_repo.rs`
- Modify: `crates/catalog/src/wall_repo.rs`
- Modify: `crates/catalog/src/generation_repo.rs`
- Modify: `crates/catalog/src/index_repo.rs`
- Modify: `crates/indexer/src/catalog_writer.rs`
- Create: `crates/catalog/tests/selection_membership.rs`
- Modify: `crates/catalog/tests/wall_query.rs`
- Modify: `crates/indexer/tests/progressive_scan.rs`

**Interfaces:**
- Consumes: `folder_groups`, `assets`, `derivatives`, `CatalogWriter::apply_batch`, `Catalog::wall_page_scoped`, and `GalleryScope`.
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderGroupRecord {
    pub id: FolderGroupId,
    pub library_id: LibraryId,
    pub relative_path: RelativePathKey,
    pub display_path: String,
}

impl Catalog {
    pub fn folder_group(&self, id: FolderGroupId) -> Result<Option<FolderGroupRecord>, CatalogError>;
    pub fn add_asset_membership(
        &mut self,
        group: FolderGroupId,
        asset: AssetId,
        generation: u64,
    ) -> Result<(), CatalogError>;
    pub fn finish_group_generation(
        &mut self,
        library: LibraryId,
        group: FolderGroupId,
        generation: u64,
    ) -> Result<(), CatalogError>;
    pub fn link_derivative_group(
        &mut self,
        derivative: DerivativeId,
        group: FolderGroupId,
    ) -> Result<(), CatalogError>;
    pub fn find_derivative_by_cache_key(
        &self,
        cache_key: &str,
    ) -> Result<Option<DerivativeRecord>, CatalogError>;
}
```

- [ ] **Step 1: Write the migration and failing repository tests**

Create migration 0009 with these concrete tables and backfills:

```sql
CREATE TABLE folder_group_assets (
  folder_group_id BLOB NOT NULL REFERENCES folder_groups(id) ON DELETE CASCADE,
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  last_seen_generation INTEGER NOT NULL,
  PRIMARY KEY(folder_group_id, asset_id)
);

INSERT INTO folder_group_assets(folder_group_id, asset_id, last_seen_generation)
SELECT folder_group_id, id, last_seen_generation
FROM assets
WHERE folder_group_id IS NOT NULL;

CREATE TABLE derivative_folder_groups (
  derivative_id BLOB NOT NULL REFERENCES derivatives(id) ON DELETE CASCADE,
  folder_group_id BLOB NOT NULL REFERENCES folder_groups(id) ON DELETE CASCADE,
  PRIMARY KEY(derivative_id, folder_group_id)
);

INSERT INTO derivative_folder_groups(derivative_id, folder_group_id)
SELECT id, folder_group_id FROM derivatives;

CREATE INDEX folder_group_assets_asset ON folder_group_assets(asset_id, folder_group_id);
CREATE INDEX derivative_folder_groups_group ON derivative_folder_groups(folder_group_id, derivative_id);
PRAGMA user_version = 9;
```

Test one asset in parent and child memberships. Assert `currentFolder` on the parent excludes a child asset, `includeSubfolders` includes it, the child selection includes it in both scopes, and finishing the parent’s next generation removes only the parent membership.

- [ ] **Step 2: Run focused catalogue/indexer tests and confirm RED**

Run: `cargo test -p photo-catalog --test selection_membership --test wall_query && cargo test -p photo-indexer --test progressive_scan`

Expected: FAIL because migration 0009 and membership methods are missing, or because existing wall queries still rely on `assets.folder_group_id`.

- [ ] **Step 3: Make selection membership authoritative**

Change `wall_page_scoped` and `wall_records_for_assets_scoped` to join `folder_group_assets fga ON fga.asset_id = assets.id` and constrain `fga.folder_group_id = ?1`. Preserve the current video exclusion, shape requirements, keyset ordering, and `relative_parent_key` current-folder predicate.

When `CatalogWriter` applies a `Discovered` event with `asset.folder_group_id`, upsert the asset and then upsert membership using the writer generation. `finish_group_generation` deletes only stale rows for that group. It marks an asset missing only when the source is online and the asset has no remaining group membership. Offline completion leaves rows and membership intact.

On derivative insert or reuse, add `derivative_folder_groups`. Change cache eviction queries so a physical derivative is reclaimable only when none of its linked groups is protected; delete the cache row only after its last group link is removed. Keep one immutable file and one derivative row per cache key.

- [ ] **Step 4: Verify overlapping selections, migration, and desktop wall behavior**

Run: `cargo test -p photo-catalog && cargo test -p photo-indexer && cargo test -p photo-app-service --test progressive_wall`

Expected: PASS. Existing pre-0009 test databases migrate and keep their original wall membership.

- [ ] **Step 5: Commit the catalogue checkpoint**

```bash
git add crates/catalog crates/indexer
git commit -m "feat: isolate overlapping gallery selections"
```

---

### Task 3: Selection-explicit gallery engine and shared runtimes

**Demo checkpoint:** Two concurrent Rust clients select different folders and receive independent progressive walls; two clients selecting the same folder share one scan.

**Files:**
- Create: `crates/app-service/src/gallery.rs`
- Create: `crates/app-service/src/hosted_runtime.rs`
- Modify: `crates/app-service/src/lib.rs`
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/app-service/src/scan.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/derivative_coordinator.rs`
- Create: `crates/app-service/tests/hosted_selections.rs`
- Create: `crates/app-service/tests/hosted_runtime.rs`
- Modify: `crates/app-service/tests/task7_source_safety.rs`

**Interfaces:**
- Consumes: Task 2 membership queries, `IndexScheduler`, `DerivativeCoordinator`, `WallUpdate`, and current scan/derivative worker code.
- Produces:

```rust
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GallerySelection {
    id: String,
    library_id: LibraryId,
    group_id: FolderGroupId,
    relative_folder: RelativePathKey,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSummary {
    pub id: String,
    pub source_id: String,
    pub display_name: String,
    pub breadcrumbs: Vec<FolderBreadcrumb>,
    pub availability: SourceAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderBreadcrumb {
    pub name: String,
    pub path: String,
}

#[derive(Clone)]
pub struct GalleryEngine {
    state: Arc<Mutex<GalleryState>>,
    scheduler: Arc<IndexScheduler>,
    runtimes: Arc<TokioMutex<HashMap<FolderGroupId, Weak<SelectionRuntime>>>>,
    catalog_path: PathBuf,
    cache_root: PathBuf,
    cache_budget: CacheBudget,
    protected_groups: ProtectedGroups,
    metadata_reader: ReaderAdapter,
}

struct GalleryState {
    libraries: LibraryService<RealSourceFs>,
    hosted_library_id: LibraryId,
}

struct SelectionRuntime {
    selection: GallerySelection,
    scan_cancel: TokioMutex<Option<watch::Sender<bool>>>,
    coordinator: Arc<DerivativeCoordinator>,
    updates: broadcast::Sender<SequencedWallUpdate>,
    history: TokioMutex<VecDeque<SequencedWallUpdate>>,
    next_event_id: AtomicU64,
    client_demand: TokioMutex<HashMap<String, ClientDemand>>,
}

struct ClientDemand {
    scope: GalleryScope,
    interaction: InteractionState,
    lease_until: tokio::time::Instant,
}

pub struct SelectionEventSubscription {
    backlog: VecDeque<SequencedWallUpdate>,
    receiver: broadcast::Receiver<SequencedWallUpdate>,
    runtime: Arc<SelectionRuntime>,
    client_id: String,
    scope: GalleryScope,
}

impl SelectionEventSubscription {
    pub async fn recv(&mut self) -> Option<SequencedWallUpdate>;
}

impl GalleryEngine {
    pub fn open(config: AppConfig, source_root: PathBuf) -> Result<Self, AppServiceError>;
    pub async fn select_relative(&self, relative: &Path) -> Result<SelectionSummary, AppServiceError>;
    pub fn resolve_selection(&self, id: &str) -> Result<GallerySelection, AppServiceError>;
    pub fn selection_summary(
        &self,
        selection: &GallerySelection,
    ) -> Result<SelectionSummary, AppServiceError>;
    pub async fn query_wall(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: WallQueryRequest,
    ) -> Result<WallPage, AppServiceError>;
    pub async fn ensure_running(&self, selection: &GallerySelection) -> Result<(), AppServiceError>;
    pub fn subscribe(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
    ) -> SelectionEventSubscription;
    pub async fn update_client_interaction(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        state: InteractionState,
    ) -> Result<bool, AppServiceError>;
}
```

The stable ID format is `selection-<folder-group UUID>`. Parsing validates the prefix and UUID, then resolves the group through SQLite. It never accepts a path.

Update cursor functions to bind the scope as well as the stable selection:

```rust
pub fn encode_cursor(
    direction: SortDirection,
    scope: GalleryScope,
    selection: &GallerySelection,
    key: &WallCursorKey,
) -> Result<String, AppServiceError>;

pub fn decode_cursor(
    value: &str,
    direction: SortDirection,
    scope: GalleryScope,
    selection: &GallerySelection,
    order: WallOrder,
) -> Result<WallCursorKey, AppServiceError>;
```

- [ ] **Step 1: Write failing multi-selection runtime tests**

Build a source fixture with `Parent/a.jpg` and `Parent/Child/b.jpg`. In one Tokio test, select `Parent` and `Parent/Child`, start both scans, collect bounded updates, and assert each event carries only its own stable selection ID. Reopen `GalleryEngine` against the same data/cache directories and assert both IDs resolve unchanged.

In a second test, call `ensure_running` concurrently for the same selection and use a counting metadata reader:

```rust
let (left, right) = tokio::join!(
    engine.ensure_running(&selection),
    engine.ensure_running(&selection),
);
left.unwrap();
right.unwrap();
assert_eq!(engine.runtime_count_for_test(), 1);
assert_eq!(reader.scan_starts(), 1);
```

Add a same-selection mixed-scope test. Subscribe client A with `CurrentFolder` and client B with `IncludeSubfolders`; assert A never receives the child asset in `catalogBatch`, derivative-ready, or asset-warning events, B does receive it, and the runtime reports `IncludeSubfolders` as the aggregate background demand. Drop B and assert the aggregate returns to `CurrentFolder` after its admitted work drains.

Add replay tests for both sides of the retained interval. An ID older than history and an ID greater than the current head each produce one fresh `resyncRequired`; an ID inside retained history replays only later events. Add a regression test proving `AppService::start_scan`, `query_wall`, `update_gallery_scope`, and its existing update subscription still behave through the desktop active-selection wrapper.

- [ ] **Step 2: Run focused tests and confirm RED**

Run: `cargo test -p photo-app-service --test hosted_selections --test hosted_runtime`

Expected: compilation fails because `GalleryEngine`, `GallerySelection`, and `SelectionEventSubscription` do not exist.

- [ ] **Step 3: Extract selection-explicit operations**

Move selection-independent catalogue, scheduler, cache, scan, and derivative state behind a shared `GalleryEngine`. On first open, add or resolve one configured library whose canonical root is the configured hosted source. Reopening the same catalog must reuse its library ID. `SelectionSummary` uses the structured `FolderBreadcrumb { name, path }` shape also exposed by the folder API; paths are mount-relative and never contain the configured native root. `selection_summary` is the sole operation used to summarize a resolved ID, so `GET /selections/{id}` can return exactly the same wire shape as selection creation.

A `SelectionRuntime` owns one scan cancellation sender, one derivative coordinator state, a `broadcast::Sender<SequencedWallUpdate>` of capacity 256, a 256-entry `VecDeque` replay history, and client demand. Store runtimes in `Arc<TokioMutex<HashMap<FolderGroupId, Weak<SelectionRuntime>>>>` so identical folders share work and unused runtimes can drain and disappear. `subscribe` copies history entries newer than `after_event_id`; if that ID predates retained history or is greater than the current event head, its backlog is one fresh `resyncRequired`. The subscription keeps the runtime alive, registers the client's requested scope as the authoritative connected-client background demand, and its `Drop` removes the client lease and demand entry. `IncludeSubfolders` dominates the aggregate while any live subscription requests it. After the last subscriber drops, let admitted work finish, cancel idle prefetch, persist all completed catalogue/cache work, and remove the registry entry when no strong runtime owner remains.

`SelectionEventSubscription::recv` drains filtered replay entries first and then live entries. Before an asset-bearing update leaves that method, filter it through Task 2 membership and the subscription's scope. This applies to `catalogBatch`, `derivativesReady`, and asset-specific warning/clear events. Selection-wide progress, settlement, source availability, and resync events remain shared. On broadcast lag, `recv` returns one fresh `resyncRequired` before resuming live entries. Do not expose the raw receiver or rely on the HTTP adapter to filter: `WallAsset` intentionally contains no native or relative path.

`update_client_interaction` refreshes state, scope, and the 30-second deadline only when `client_id` has a live subscription, returning `true` when it updated that client and `false` for an unknown or already-dropped client. This makes subscription drop authoritative when a heartbeat races with disconnect.

Use one shared `IndexScheduler` for every runtime. Keep the first-visible, near-viewport, and idle-background priorities already used by the desktop service. Scan the selected directory recursively regardless of scope. Publish each `WallUpdate` through:

```rust
pub struct SequencedWallUpdate {
    pub id: u64,
    pub update: WallUpdate,
}
```

with a per-runtime `AtomicU64`.

Make current desktop methods thin wrappers that resolve the persisted desktop selection and persisted desktop gallery scope, then call the explicit engine operation. Do not change Tauri command names or DTO casing.

- [ ] **Step 4: Verify concurrency, restart stability, and desktop regression**

Run: `cargo test -p photo-app-service && cargo test --workspace`

Expected: PASS. The shared-selection test reports one runtime and one scan start; different selections progress independently.

- [ ] **Step 5: Commit the runtime checkpoint**

```bash
git add crates/app-service
git commit -m "feat: run hosted galleries by explicit selection"
```

---

### Task 4: Gallery HTTP API, bounded SSE, and client interaction leases

**Demo checkpoint:** `curl` can create a selection, page its wall, request scan work, and watch progressive updates over SSE. A second selection remains unaffected.

**Files:**
- Modify: `crates/server/Cargo.toml`
- Modify: `crates/server/src/lib.rs`
- Modify: `crates/server/src/api/mod.rs`
- Modify: `crates/server/src/api/error.rs`
- Modify: `crates/server/src/api/types.rs`
- Create: `crates/server/src/api/gallery.rs`
- Create: `crates/server/src/api/events.rs`
- Create: `crates/server/tests/gallery_api.rs`
- Create: `crates/server/tests/events_api.rs`
- Modify: `crates/app-service/src/hosted_runtime.rs`
- Modify: `crates/app-service/tests/hosted_runtime.rs`

**Interfaces:**
- Consumes: Task 1 `ContainedFolderRoot`, Task 3 `GalleryEngine`, `SelectionSummary`, `SelectionEventSubscription`.
- Produces the approved `/api/v1/selections` routes and:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSelectionRequest { path: String }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WallParams {
    scope: GalleryScope,
    direction: SortDirection,
    cursor: Option<String>,
    limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InteractionRequest {
    client_id: String,
    scope: GalleryScope,
    state: InteractionState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsParams {
    client_id: String,
    scope: GalleryScope,
    after_event_id: Option<u64>,
}
```

- [ ] **Step 1: Write failing route, cursor, event, and lease tests**

Use `tower::ServiceExt::oneshot` to assert:

- `POST /api/v1/selections` returns 201 and a stable `SelectionSummary`.
- `GET /api/v1/selections/{id}` returns the same summary after rebuilding the router.
- Wall limits 0 and 251 return `invalidLimit`; a cursor reused with a different scope or direction returns `invalidCursor`.
- Unknown selection IDs return a path-free 404.
- SSE sets `Content-Type: text/event-stream`, `Cache-Control: no-cache`, and `X-Accel-Buffering: no`.
- Reconnecting with `Last-Event-ID` continues after that ID; a lagged ID receives `resyncRequired`.
- `Last-Event-ID` takes precedence over `afterEventId` when both are present; a replay ID newer than the runtime's current head also receives `resyncRequired`.
- Two clients on the same selection but different scopes receive only events allowed by their own scope, while the runtime demand uses `includeSubfolders` until the broader subscription closes.
- An active lease from client A keeps the shared scheduler active even if client B posts idle. Closing A’s subscription or advancing paused Tokio time past 30 seconds expires the lease and resumes idle background work.
- A total query string over 8192 bytes, more than 8 decoded query parameters, repeated singleton parameters, a cursor over 2048 bytes, a selection ID over 128 ASCII bytes, a client ID over 128 ASCII bytes, or an event ID over 20 ASCII decimal bytes fails with a fixed `invalidRequest` response before database or filesystem access. Unit-test the shared identifier guard with a derivative ID over 512 decoded bytes; Task 5 asserts that guard through the derivative route after the route exists.

- [ ] **Step 2: Run server tests and confirm RED**

Run: `cargo test -p photo-server --test gallery_api --test events_api`

Expected: 404 or compilation failure for the missing selection, wall, interaction, and event handlers.

- [ ] **Step 3: Implement bounded JSON and SSE handlers**

Mount these exact routes under `/api/v1`:

```text
POST /selections
GET  /selections/{id}
GET  /selections/{id}/wall
POST /selections/{id}/interaction
GET  /selections/{id}/events
```

Once `photo-server` depends on `photo-app-service`, replace the Task 1 server-local breadcrumb definition with `pub use photo_app_service::FolderBreadcrumb` from `api/types.rs`. Folder listings and selection summaries must serialize the same `{ name, path }` wire shape.

The create-selection handler calls `GalleryEngine::ensure_running` after it resolves or creates the stable selection. `GET /selections/{id}` resolves the opaque ID and calls `GalleryEngine::selection_summary`; it never rebuilds breadcrumbs from an ID or native path. A wall or event request also calls `ensure_running` idempotently, so restoring a saved browser selection resumes work without requiring another folder selection.

Add one request-target guard shared by the folder, gallery, event, and derivative routers. Reject query text over 8192 bytes, more than 8 decoded parameters, repeated singleton parameters, folder strings over 4096 bytes, cursors over 2048 bytes, selection IDs over 128 ASCII bytes, derivative IDs over 512 decoded bytes, client IDs over 128 ASCII bytes, and event IDs over 20 ASCII decimal bytes. Reject JSON bodies over 64 KiB. Task 5 enforces derivative asset lists of 1 to 250 when that route is introduced. Apply these checks before UUID parsing, cursor decoding, catalogue lookup, or filesystem access. Never serialize an internal `AppServiceError`; map each public failure to `ApiError::new(StatusCode::BAD_REQUEST, "invalidRequest", "That request is not valid.")` or the more specific existing `invalidFolderPath`, `invalidCursor`, and `invalidLimit` codes.

Format SSE as named `wallUpdate` events with monotonic `id:` values and JSON `data:` equal to the existing `WallUpdate`. Accept `scope` and `clientId` on every event request. Accept replay position from either standard `Last-Event-ID` or browser-compatible `afterEventId`, with the header taking precedence. The handler consumes only `SelectionEventSubscription::recv`, never its internal broadcast receiver. Send a comment heartbeat every 15 seconds. A subscription drop removes that client's lease and demand entry even if an interaction heartbeat races with disconnect. The interaction handler calls `update_client_interaction`; both a refreshed live client and an unknown client return 204 so a connection-opening race does not become a user-visible error. Track deadlines using Tokio time and compute scheduler interaction as active when any non-expired client lease is active.

- [ ] **Step 4: Verify transport isolation and bounded behavior**

Run: `cargo test -p photo-server --test gallery_api --test events_api && cargo test -p photo-app-service --test hosted_runtime`

Expected: PASS, including mismatched cursor rejection, SSE lag recovery, and multi-client lease aggregation.

- [ ] **Step 5: Commit the HTTP gallery checkpoint**

```bash
git add crates/server crates/app-service
git commit -m "feat: expose selection scoped gallery events"
```

---

### Task 5: Selection-scoped derivative generation and safe HTTP delivery

**Demo checkpoint:** A wall request can generate a thumbnail, then an origin-relative derivative URL returns cached image bytes immediately without opening the source file again.

**Files:**
- Modify: `crates/cache/src/writer.rs`
- Modify: `crates/cache/tests/cache_policy.rs`
- Modify: `crates/app-service/src/gallery.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/catalog/src/cache_repo.rs`
- Modify: `crates/server/src/api/mod.rs`
- Modify: `crates/server/src/api/gallery.rs`
- Create: `crates/server/src/api/derivative.rs`
- Create: `crates/server/tests/derivative_api.rs`
- Modify: `crates/app-service/tests/hosted_selections.rs`
- Modify: `crates/app-service/tests/task7_source_safety.rs`

**Interfaces:**
- Consumes: `DerivativeRequest`, Task 2 derivative membership, and Task 3 selection runtimes.
- Produces:

```rust
impl GalleryEngine {
    pub async fn request_derivatives(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError>;

    pub fn open_derivative(&self, opaque_id: &str) -> Result<ManagedDerivative, AppServiceError>;
}

pub struct ManagedDerivative {
    pub file: std::fs::File,
    pub content_type: &'static str,
    pub content_length: u64,
    pub etag: String,
}

impl CacheWriter {
    pub fn open_checked(&self, relative_path: &Path) -> Result<std::fs::File, CacheError>;
}
```

`opaque_id` is the immutable derivative cache key already carried in `DerivativeReference.key`. The browser URL is `/api/v1/derivatives/${encodeURIComponent(reference.key)}`.

- [ ] **Step 1: Write failing derivative authorization and containment tests**

Assert that a request rejects foreign assets outside the supplied selection/scope, rejects empty or over-250 asset lists, generates wall thumbnails before screen previews, coalesces the same asset/class/cache-key across two clients, and publishes ready updates only to subscribers whose scope admits the asset. A current-folder subscriber must not receive a ready reference for a child asset requested by an include-subfolders subscriber.

At the HTTP layer assert:

```rust
assert_eq!(response.status(), StatusCode::OK);
assert_eq!(response.headers()[CONTENT_TYPE], "image/jpeg");
assert_eq!(response.headers()[CACHE_CONTROL], "public, max-age=31536000, immutable");
assert!(response.headers()[ETAG].to_str().unwrap().starts_with('"'));
assert_eq!(response.headers()["x-content-type-options"], "nosniff");
```

Insert a catalogue row whose relative cache path contains `..`, one whose file is missing, and a symlink escaping the cache root. Each must fail closed with 404 and must not read source media. Assert an opaque derivative ID over 512 decoded bytes and a request target over 8192 query bytes fail with `invalidRequest` before catalogue lookup.

- [ ] **Step 2: Run focused derivative tests and confirm RED**

Run: `cargo test -p photo-server --test derivative_api && cargo test -p photo-app-service --test hosted_selections`

Expected: missing method/route failures.

- [ ] **Step 3: Implement explicit derivative demand and managed streaming**

Validate all requested asset IDs against Task 2 membership and the requested `GalleryScope`; accept 1 to 250 IDs. The Task 4 event subscription is the authoritative source of connected-client background scope, so a foreground derivative request does not create or retain a client-demand entry. Aggregate background demand per runtime so `IncludeSubfolders` dominates while any connected event subscription asks for it. Preserve visible over near-viewport over background scheduling and wall-thumbnail-before-screen-preview ordering.

Resolve the route ID through `Catalog::find_derivative_by_cache_key`, then pass only the stored relative path to `CacheWriter::open_checked`. Recheck file metadata, derive content type from the stored derivative kind plus file signature, and stream the managed file. Never accept a source path, never use request range headers, and never fall back to the original.

Add:

```rust
POST /api/v1/selections/{id}/derivatives
GET  /api/v1/derivatives/{id}
```

The derivative request body contains a concrete scope plus the existing request, for example `{ "scope": "includeSubfolders", "request": { "assetIds": ["68c51f56-bbe6-4e6b-8ca0-e6e48f1a2cd1"], "priority": "visible", "kind": "wallThumbnail" } }`. The original implementation waited for the success or typed-failure boundary. The later connection-starvation remediation validates and admits the request before returning, coalesces work by immutable cache key across clients and scopes for the stable folder selection, compacts duplicate scope authorization markers, caps unique pending HTTP work at 1,024 jobs per selection, and reports completion or one retryable generation warning per settled failed attempt through the event stream.

- [ ] **Step 4: Verify caching, deduplication, and source safety**

Run: `cargo test -p photo-cache -p photo-catalog -p photo-app-service -p photo-server`

Expected: PASS. The source snapshot test shows identical bytes, lengths, and modified times after scanning and both derivative classes.

- [ ] **Step 5: Commit the derivative checkpoint**

```bash
git add crates/cache crates/app-service crates/catalog crates/server
git commit -m "feat: serve managed hosted photo derivatives"
```

---

### Task 6: HTTP PhotoService and browser-owned preferences

**Demo checkpoint:** Two isolated browser storage contexts restore different selections, appearance, scope, and sort direction while sharing the same server.

**Files:**
- Modify: `apps/interface/src/services/photoService.ts`
- Create: `apps/interface/src/services/browserPreferences.ts`
- Create: `apps/interface/src/services/browserPreferences.test.ts`
- Create: `apps/interface/src/services/httpPhotoService.ts`
- Create: `apps/interface/src/services/httpPhotoService.test.ts`
- Modify: `apps/interface/src/services/inMemoryPhotoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.test.ts`
- Modify: `apps/interface/src/app/usePhotoWall.ts`
- Modify: `apps/interface/src/components/PhotoWall.browser.test.tsx`
- Modify: `apps/interface/src/wall/wallReducer.ts`
- Modify: `apps/interface/src/wall/wallReducer.test.ts`
- Modify: `apps/interface/src/main.tsx`
- Modify: `apps/interface/vite.config.ts`

**Interfaces:**
- Consumes: Task 4/5 JSON, SSE, and derivative routes.
- Produces additions to the host-neutral interface:

```ts
export interface FolderBreadcrumb { name: string; path: string }
export interface FolderEntry { name: string; path: string }
export interface FolderListing {
	path: string;
	breadcrumbs: FolderBreadcrumb[];
	children: FolderEntry[];
}

export interface FolderBrowserState {
	breadcrumbs: FolderBreadcrumb[];
	initialPath: string;
}

export interface PhotoServiceCapabilities {
	chooseFolder: boolean;
	folderSelection: "native" | "hosted";
	locateFolder: boolean;
}

export interface PhotoService {
	readonly capabilities: PhotoServiceCapabilities;
	getBootstrapState(): Promise<BootstrapState>;
	chooseFolder(): Promise<ChooseFolderResult>;
	updateAppearance(appearance: Appearance): Promise<BootstrapState>;
	updateGalleryScope(scope: GalleryScope): Promise<BootstrapState>;
	queryWall(request: WallQueryRequest): Promise<WallPage>;
	requestDerivatives(request: DerivativeRequest): Promise<void>;
	setWallInteraction(active: boolean): Promise<void>;
	watchWallUpdates(listener: (update: WallUpdate) => void): () => void;
	derivativeUrl(reference: DerivativeReference): string;
	listFolders(path: string): Promise<FolderListing>;
	selectFolder(path: string): Promise<ChooseFolderResult>;
	folderBrowserState(): FolderBrowserState;
	initialSortDirection(): SortDirection;
	rememberSortDirection(direction: SortDirection): void;
}
```

- [ ] **Step 1: Write failing storage and adapter tests**

Use injected `Storage`, `fetch`, `EventSource`, and UUID factories. Assert local storage key `photo-viewer.hosted.v1` contains only:

```ts
{
	selectionId: string | null,
	breadcrumbs: FolderBreadcrumb[],
	appearance: Appearance,
	galleryScope: GalleryScope,
	sortDirection: SortDirection,
}
```

Assert session key `photo-viewer.client.v1` is reused within a tab and differs across injected session stores. Test malformed JSON and unknown enum values falling back to system/include-subfolders/oldest-first without throwing.

For the adapter, assert origin-relative calls, encoded paths, fixed public error mapping, selection IDs and scope on every wall/derivative/interaction/event request, `afterEventId` reconnection, one authoritative wall resync on `resyncRequired`, and `derivativeUrl({ assetId: "asset-a", kind: "wallThumbnail", key: "cache/key" }) === "/api/v1/derivatives/cache%2Fkey"`.

Return selection summaries with structured `{ name, path }` breadcrumbs. After a successful `selectFolder("Trips/Iceland")`, assert the adapter atomically stores its returned selection ID and breadcrumbs. When `GET /selections/{id}` returns 404, assert bootstrap clears only the invalid ID, retains the breadcrumbs, and exposes `folderBrowserState()` with `initialPath === "Trips/Iceland"`.

Use fake timers to assert `setWallInteraction(true)` posts immediately and every 10 seconds while active, each post includes the current scope, and `setWallInteraction(false)`, stream disposal, or selection change cancels the timer. When an event stream reaches `open` while interaction is active, assert the adapter refreshes the lease immediately to close the connection-opening race. Reconnecting an event stream repeats selection and scope. Changing scope starts a distinct replay position rather than reusing the previous scope's event ID.

Restore `newestFirst` from local storage and assert the first wall query uses `newestFirst`. Repeat after `resetSource` and assert the reducer preserves the accepted direction.

- [ ] **Step 2: Run interface unit tests and confirm RED**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/browserPreferences.test.ts src/services/httpPhotoService.test.ts src/wall/wallReducer.test.ts && npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PhotoWall.browser.test.tsx`

Expected: missing modules/types and failing `PhotoService` conformance.

- [ ] **Step 3: Implement preferences, HTTP mapping, and hosted entry selection**

Implement strict JSON decoding with a versioned storage key. `getBootstrapState` calls server bootstrap, resolves the saved selection with `GET /selections/{id}`, and combines that source summary with local appearance/scope. Selection responses use the same structured `FolderBreadcrumb` shape as folder listings. `selectFolder` stores the returned stable selection ID and breadcrumbs in one preference write before returning its selected state. On 404, retain breadcrumbs for folder-browser recovery but clear the invalid selection ID. `folderBrowserState()` returns a defensive copy of those breadcrumbs and uses the last breadcrumb path as `initialPath`, falling back to the empty mounted-root path.

The Tauri and in-memory adapters implement the expanded contract explicitly. They return `{ breadcrumbs: [], initialPath: "" }` from `folderBrowserState`, return `oldestFirst` from `initialSortDirection`, keep `rememberSortDirection` in adapter memory, and reject `listFolders`/`selectFolder` with `new PhotoServiceError("unsupportedCapability", "This host uses its system folder picker.")`; their capability prevents those methods from being called by the interface.

`watchWallUpdates` creates an `EventSource` URL with `clientId`, scope, and the current selection. Because native `EventSource` cannot set `Last-Event-ID` manually, reconnect through a query parameter `afterEventId` that the server treats the same as the header. Associate the active replay ID with its `(selectionId, scope)` and clear it when either value changes; only a reconnect for the same pair reuses it. Use capped exponential delays from 250 ms to 5 seconds. Keep the current wall during disconnect. Deliver one `resyncRequired` per detected gap.

`setWallInteraction(true)` posts immediately, then refreshes the active lease every 10 seconds while interaction remains active. Each request includes `clientId`, the current scope, and `state: "active"`. If the stream's `open` event fires while interaction is active, post another active refresh immediately. `setWallInteraction(false)` cancels the timer and posts `state: "idle"`. Event-stream disposal, selection change, and adapter disposal cancel the timer; the server still treats subscription drop as authoritative if a heartbeat races with disconnect.

Initialize `usePhotoWall` with `service.initialSortDirection()` rather than the module-level oldest-first state. Change `resetSource` to preserve the current accepted direction, remove the `initialSourceQuery ? "oldestFirst"` override, and make the first wall request use reducer state. Call `service.rememberSortDirection(direction)` only after the reducer accepts a new direction. Do not put sort in SQLite.

Select adapters explicitly in `main.tsx`:

```ts
const service = import.meta.env.MODE === "memory"
	? createInMemoryPhotoService({ cancelFolderPicker: true })
	: import.meta.env.MODE === "hosted"
		? createHttpPhotoService()
		: createTauriPhotoService();
```

- [ ] **Step 4: Verify all adapters and desktop type compatibility**

Run: `npm run typecheck && npm test && npm run test:browser && cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`

Expected: PASS. Tauri still uses native selection and hosted state never invokes a Tauri API.

- [ ] **Step 5: Commit the browser-service checkpoint**

```bash
git add apps/interface
git commit -m "feat: connect browser galleries over HTTP"
```

---

### Task 7: Responsive hosted folder browser and complete web flow

**Demo checkpoint:** On desktop and phone widths, the Folders control opens a contained browser, selects a directory, shows its justified wall, and opens the existing viewer without changing another browser context.

**Files:**
- Create: `apps/interface/src/components/HostedFolderBrowser.tsx`
- Create: `apps/interface/src/components/HostedFolderBrowser.browser.test.tsx`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/components/NavigationRail.tsx`
- Modify: `apps/interface/src/components/App.browser.test.tsx`
- Modify: `apps/interface/src/app/useAppController.ts`
- Modify: `apps/interface/src/styles/appShell.module.css`
- Modify: `apps/interface/src/styles/tokens.css`

**Interfaces:**
- Consumes: Task 6 `folderSelection`, `listFolders`, and `selectFolder`.
- Produces:

```tsx
interface HostedFolderBrowserProps {
	initialBreadcrumbs: FolderBreadcrumb[];
	onClose(): void;
	onSelected(result: ChooseFolderResult): void;
	service: PhotoService;
}
```

- [ ] **Step 1: Write failing responsive browser-flow tests**

At 1440×1024, assert the hosted Folders button opens a centred modal with breadcrumbs, Back, child directory buttons, Retry, and Open this folder. At 390×844 and coarse-pointer emulation, assert the same control opens a full-height sheet within safe-area insets.

Test keyboard focus enters the dialog, remains contained through Tab/Shift-Tab, Escape closes and restores the Folders trigger, loading sets `aria-busy`, and a failed child request leaves the last good listing visible with an alert and Retry. Test reduced motion and run `axe` with zero serious/critical violations.

Add a service fixture that returns `Trips -> Iceland -> Processed`. Select `Iceland`, assert the dialog closes and source title updates, then reopen and assert breadcrumbs resume there. Add an invalid-saved-selection fixture whose retained breadcrumbs are `Trips`, `Iceland`, `Processed`; make `Processed` fail, `Iceland` succeed, and assert recovery tries those mount-relative breadcrumb paths deepest-first without slicing strings or exposing `/photos`.

- [ ] **Step 2: Run the browser tests and confirm RED**

Run: `npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/HostedFolderBrowser.browser.test.tsx`

Expected: FAIL because the hosted dialog and capability branch do not exist.

- [ ] **Step 3: Implement the dialog/sheet and shell wiring**

Render `HostedFolderBrowser` only when `capabilities.folderSelection === "hosted"`; keep native `chooseFolder()` unchanged. Load one directory level on open and on child navigation. Derive Back from returned breadcrumbs, not string slicing. Disable Open while a request is in flight, and close only after `selectFolder` returns `{ kind: "selected" }`.

Use the existing spacing, colour, typography, icon, focus, and safe-area tokens. The desktop modal is at most 640 px wide and 70 vh tall. Under 640 px or `(pointer: coarse)`, use the viewport minus safe-area insets. Directory rows have at least a 44 px target. Suppress nonessential motion under `prefers-reduced-motion`.

When AppShell opens the hosted picker, read `service.folderBrowserState()` and pass its breadcrumbs as `initialBreadcrumbs`. `HostedFolderBrowser` tries those structured breadcrumb paths from deepest to shallowest and opens the first whose `listFolders` call succeeds. If none succeeds, it lists the mounted root with the empty path. Use only each breadcrumb's returned `path`; do not reconstruct it from names or import browser preferences into React. Never show a Locate Folder action in hosted mode.

- [ ] **Step 4: Verify responsive UI and existing viewer behavior**

Run: `npm run typecheck && npm run test:browser`

Expected: PASS for the new dialog plus existing wall, viewer, contrast, motion, zoom, pan, and rotation tests.

- [ ] **Step 5: Commit the complete browser flow**

```bash
git add apps/interface
git commit -m "feat: choose hosted gallery folders responsively"
```

---

### Task 8: Static host, security headers, and production server startup

**Demo checkpoint:** One `photo-server` process serves `/`, a nested interface route, API JSON, SSE, derivatives, and health with no absolute URL generation.

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/server/Cargo.toml`
- Modify: `crates/server/src/lib.rs`
- Modify: `crates/server/src/main.rs`
- Create: `crates/server/src/static_host.rs`
- Create: `crates/server/tests/static_host.rs`
- Modify: `crates/server/tests/health_api.rs`
- Modify: `package.json`
- Modify: `apps/interface/package.json`

**Interfaces:**
- Consumes: built `apps/interface/dist`, all `/api/v1` routes, and `/healthz`.
- Produces: `build_router(state, web_root)` with API/health routes taking precedence over static fallback.

- [ ] **Step 1: Write failing static-host and security tests**

Create a temporary web root containing `index.html`, `assets/app-immutable.js`, and `missing.txt`. Assert:

- `/` and `/gallery/selection-id` return `index.html` without a redirect.
- `/assets/app-immutable.js` returns the asset with a long immutable cache policy.
- `/missing.txt` returns 404 rather than `index.html` because it looks like a file request.
- `/api/v1/unknown` returns JSON 404 rather than the interface shell.
- interface responses contain `Content-Security-Policy`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and `frame-ancestors 'none'`.
- no `Location` header or response body contains `http://`, `https://`, `Host`, or a forwarded hostname supplied by the test.

- [ ] **Step 2: Run server tests and confirm RED**

Run: `cargo test -p photo-server --test static_host --test health_api`

Expected: nested routes return 404 and security/cache headers are absent.

- [ ] **Step 3: Implement the static fallback and real startup composition**

Enable only the required `tower-http` `fs`, `set-header`, `catch-panic`, and `trace` features. Serve real files from `ServerConfig::web_root`; use `index.html` only for extensionless, non-API GET/HEAD paths. Add a CSP permitting only same-origin scripts, styles, images, fetch/event streams, and inline style attributes already required by photo layout calculations:

```text
default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'
```

Open `GalleryEngine` with `AppConfig::new(data_dir, cache_dir)` plus the configured source root. Keep health at `/healthz`. Log forwarded host/scheme/client address only as untrusted text; never feed them into a URL, path, authorization, or containment decision.

Add root scripts:

```json
{
  "web:build": "npm run build --workspace @photo-viewer/interface -- --mode hosted",
  "web:dev": "npm run dev --workspace @photo-viewer/interface -- --mode hosted"
}
```

- [ ] **Step 4: Verify the composed server and frontend build**

Run: `npm run web:build && cargo test -p photo-server && cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: PASS. `apps/interface/dist/index.html` contains origin-relative asset URLs and the router serves it.

- [ ] **Step 5: Commit the production-process checkpoint**

```bash
git add Cargo.toml Cargo.lock package.json apps/interface crates/server
git commit -m "feat: serve the hosted photo viewer securely"
```

---

### Task 9: OCI image, deployment examples, and hosted acceptance

**Demo checkpoint:** Build with Podman, browse two independent folders in two browser contexts, restart with retained volumes, and view cached photos after the source is unavailable.

**Files:**
- Create: `Containerfile`
- Create: `.containerignore`
- Create: `deploy/compose.yaml`
- Create: `deploy/nginx.conf.example`
- Create: `docs/deployment/hosted.md`
- Create: `tests/hosted/playwright.config.ts`
- Create: `tests/hosted/hosted.spec.ts`
- Create: `scripts/hosted-smoke.sh`
- Modify: `package.json`
- Modify: `package-lock.json`
- Modify: `README.md`

**Interfaces:**
- Consumes: Tasks 1 through 8 and runtime environment variables from the spec.
- Produces: image `localhost/photo-viewer:dev`, port 8080, non-root UID/GID 10001, read-only `/photos`, and persistent data/cache volumes.

- [ ] **Step 1: Write the failing hosted acceptance test and source snapshot helper**

Add `@playwright/test` version `1.62.1` as a direct dev dependency and add `test:hosted` to the root scripts now so the RED command invokes the real test. The test receives `PHOTO_VIEWER_BASE_URL`, `PHOTO_VIEWER_PHASE`, and `PHOTO_VIEWER_STATE_DIR`.

In `beforeRestart`, open two browser contexts with separate local/session storage, select `A` and `B`, choose different appearance and sort values, keep A on `currentFolder`, and put B on `includeSubfolders`. Give A one child photo that remains outside its current-folder derivative demand. Assert source titles, first wall filenames, viewer, filmstrip, zoom, and pan remain independent. Save each context's Playwright storage state to the temporary state directory and record its opaque selection ID plus one cached derivative URL and ETag in a temporary JSON record.

In `afterRestart`, create new contexts from those two saved storage states. Assert each browser restores its own folder, appearance, scope, sort direction, first wall row, and cached viewer without selecting a folder again. Assert the recorded selection IDs still resolve and cached derivative requests retain their ETags. New tabs receive new session client IDs; restoration depends only on local storage.

In `offline`, reopen both saved contexts after source availability has been refreshed. Assert cached wall and viewer content remain usable. Switch A to `includeSubfolders`, which must query its already-recursive catalogue without walking the source, then assert the known child photo has the unavailable cue and cannot open because its derivative was never requested.

The shell smoke script creates a temporary tree with deterministic JPEG fixtures and computes a manifest before startup:

```sh
find "$source_dir" -type f -exec stat -f '%N|%z|%m' {} \; | LC_ALL=C sort
find "$source_dir" -type f -exec shasum -a 256 {} \; | LC_ALL=C sort
```

Use GNU `stat -c` when `stat -f` is unavailable. Compare both manifests after folder browsing, scan, sort/scope changes, viewer requests, restart, traversal probes, and offline reads.

- [ ] **Step 2: Run the acceptance entry point and confirm RED**

Run: `npm run test:hosted`

Expected: FAIL because no hosted server is running at the required base URL.

- [ ] **Step 3: Build the standard OCI image and deployment files**

Use these stages and responsibilities:

```Dockerfile
FROM node:24-bookworm-slim AS web
WORKDIR /build
COPY package.json package-lock.json ./
COPY apps/interface/package.json apps/interface/package.json
COPY apps/desktop/package.json apps/desktop/package.json
RUN npm ci
COPY apps/interface apps/interface
RUN npm run web:build

FROM rust:1.97.1-bookworm AS rust
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --locked --release -p photo-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 photo-viewer \
    && useradd --uid 10001 --gid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin photo-viewer \
    && install -d -o 10001 -g 10001 /var/lib/photo-viewer /var/cache/photo-viewer /app/web
COPY --from=rust /build/target/release/photo-server /usr/local/bin/photo-server
COPY --from=web /build/apps/interface/dist /app/web
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/photo-server"]
```

The Compose-compatible file mounts `${PHOTO_PATH:-/srv/photos}:/photos:ro`, named `photo-viewer-data` and `photo-viewer-cache` volumes, binds `127.0.0.1:8080:8080`, sets the four approved environment variables, and uses `restart: unless-stopped`.

The Nginx example proxies to `127.0.0.1:8080`, forwards `Host`, `X-Forwarded-Host`, `X-Forwarded-Proto`, and `X-Forwarded-For`, disables proxy buffering, and sets `proxy_read_timeout 1h`. It does not configure a public-URL environment variable.

- [ ] **Step 4: Implement automated Podman lifecycle acceptance**

`scripts/hosted-smoke.sh` must:

1. create controlled source/data/cache directories;
2. build `localhost/photo-viewer:dev` with `podman build`;
3. run with `/photos:ro,Z`, writable retained volumes, and `127.0.0.1::8080`;
4. discover the mapped port and wait up to 60 seconds for `/healthz`;
5. run Playwright with `PHOTO_VIEWER_PHASE=beforeRestart`, storing both contexts under the smoke test's temporary state directory;
6. stop and recreate the container with the same data/cache, then wait for `/healthz` again;
7. run Playwright with `PHOTO_VIEWER_PHASE=afterRestart` and the saved storage states, proving browser restoration as well as stable selection IDs and derivative ETags;
8. while the restarted container remains running, make the mounted source unreadable, trigger a folder-list or scan request so availability is refreshed, confirm degraded health, then run `PHOTO_VIEWER_PHASE=offline` to verify cached wall/viewer access and the uncached unavailable behavior before restoring access for cleanup;
9. probe `%2e%2e`, absolute, NUL-encoded, over-limit, repeated-parameter, file, and symlink escape folder paths;
10. compare source metadata and hash manifests after every phase;
11. remove only its named temporary container, network, volumes, browser-state directory, and record files through an EXIT trap.

Use explicit generated names prefixed `photo-viewer-smoke-`; never prune global Podman state.

- [ ] **Step 5: Document deployment and reverse-proxy behavior**

`docs/deployment/hosted.md` must include Podman and Docker build/run commands, Compose usage, the `/photos` read-only contract, UID 10001 bind-mount permissions, named-volume persistence, health checking, backup of `catalog.sqlite`, cache rebuild expectations, Nginx config, restart/upgrade steps, and the fact that `https://photos.docker.jenner.lan` needs no hostname setting because every URL is origin-relative.

Update `README.md` with links to desktop development and hosted deployment. Keep the script introduced by the acceptance test:

```json
{
  "test:hosted": "playwright test --config tests/hosted/playwright.config.ts"
}
```

- [ ] **Step 6: Run the pre-commit verification gate**

Run these commands from the feature worktree with fresh output:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
npm run check
npm run typecheck
npm test
npm run test:browser
podman build -t localhost/photo-viewer:dev -f Containerfile .
./scripts/hosted-smoke.sh
```

Expected: every command exits 0; the smoke test reports two-browser independence, browser-state restoration after restart, stable derivative ETags, offline cached viewing, traversal and limit rejection, and an unchanged source manifest. Task 9 files remain intentionally uncommitted until Step 7.

- [ ] **Step 7: Commit deployment and acceptance artifacts**

```bash
git add Containerfile .containerignore deploy docs/deployment tests/hosted scripts/hosted-smoke.sh package.json package-lock.json README.md
git commit -m "feat: package hosted photo viewer for OCI"
```

- [ ] **Step 8: Request code review and prepare the user demo**

Use `superpowers:requesting-code-review` against the full branch diff from its merge base with `codex/include-subfolders-gallery`. Fix Critical and Important findings through `superpowers:receiving-code-review` and rerun each focused test named by the affected task. Stage only tracked fixes plus new files under the already approved feature paths, then commit review fixes before the final gate:

```bash
git add -u
git add apps/interface crates tests/hosted scripts/hosted-smoke.sh Containerfile .containerignore deploy docs/deployment README.md package.json package-lock.json Cargo.toml Cargo.lock
git diff --cached --quiet || git commit -m "fix: address hosted web review"
```

Run the final gate from the committed tree with fresh output:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
npm run check
npm run typecheck
npm test
npm run test:browser
podman build -t localhost/photo-viewer:dev -f Containerfile .
./scripts/hosted-smoke.sh
git status --short
```

Expected: every command exits 0, the hosted smoke report repeats the complete acceptance evidence, and `git status --short` prints nothing. Start the accepted image locally and provide the exact mapped loopback URL printed by `podman port`, such as `http://127.0.0.1:49152`, for exploratory testing. Do not merge or push until the user approves the demo.
