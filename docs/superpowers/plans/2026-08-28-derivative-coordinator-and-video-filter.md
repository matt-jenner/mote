# Derivative Coordinator and Video Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the race-prone preview gates with one bounded thumbnail-first coordinator, keep videos indexed but invisible, and make wall tiles open only after their current thumbnail is visibly painted.

**Architecture:** A new `DerivativeCoordinator` owns derivative job identity, priority promotion, bounded recent state, background generations, foreground waiters, collection phase, and commit admission for the active selection. Catalogue queries expose only still images to photo surfaces while retaining video rows internally. Image encoding stays outside the coordinator lock; a linearizable `CommitPermit` is issued under the same state boundary used by invalidation, so admitted commits are ordered before later requests and rejected jobs have no cache, eviction, catalogue, warning, or publication side effects.

**Tech Stack:** Rust 1.97.1, edition 2024, Tokio 1.53.1, SQLite through `photo-catalog`, `image` 0.25.10, React 19.2.8, TypeScript 7.0.2, Vitest 4.1.11, Playwright WebKit, CSS Modules, Tauri 2.11.5.

**Spec:** `docs/superpowers/specs/2026-08-28-derivative-coordinator-and-video-filter-design.md`

## Global Constraints

- Source media is read-only. Production code may read sources but never write, rename, move, copy, or delete them.
- SQLite stores metadata and derivative references, never image blobs. Generated images stay inside the managed cache.
- Videos remain indexed but are absent from every user-facing photo item, count, progress total, sequence, search projection, empty-state decision, and derivative queue.
- Wall thumbnails remain durable 1024-pixel long-edge derivatives. Screen previews remain non-durable 4096-pixel long-edge derivatives capped at source dimensions.
- All catalogue and cache work remains scoped by active `SelectionToken`, folder group, immutable derivative key, and source fingerprint.
- Collection traversal and the recent-priority window are bounded at 250 assets.
- Background screen previews begin only after every eligible still in the active folder group is thumbnail-ready or terminal for its current derivative key.
- An explicit viewer preview may run after its own thumbnail is ready; it does not wait for the collection background phase.
- React remains host-neutral. Only `apps/interface/src/services/tauriPhotoService.ts` imports Tauri APIs.
- The immersive viewer remains intentionally dark in system, light, and dark application appearances.
- Existing multiple-root identity, offline catalogue behavior, whole-group screen-preview eviction, metadata precedence, source-path opacity, and cache containment remain intact.
- Every production change follows strict red-green-refactor. Record the failing command and expected behavioral failure before implementation.
- Each task ends with focused verification, a fresh Sol review, and one logical commit or a small explicitly related commit series. Do not merge or push.

---

### Task 1: Make the catalogue projection photo-only and persist terminal derivative outcomes

**Files:**
- Create: `crates/catalog/migrations/0007_derivative_coordinator.sql`
- Create: `crates/catalog/src/derivative_failure_repo.rs`
- Modify: `crates/catalog/src/lib.rs`
- Modify: `crates/catalog/src/wall_repo.rs`
- Modify: `crates/indexer/src/scanner.rs`
- Test: `crates/catalog/tests/wall_query.rs`
- Test: `crates/catalog/tests/catalog_round_trip.rs`
- Test: `crates/indexer/tests/progressive_scan.rs`
- Test: `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**
- Consumes: existing `assets.media_kind`, `MediaKind::Video`, `WallOrder`, `DerivativeKey`, and scan `DiscoveredAsset` records.
- Produces: `Catalog::photo_asset_ids_page`, `Catalog::find_terminal_derivative_failure`, `Catalog::record_terminal_derivative_failure`, `Catalog::clear_terminal_derivative_failure`, and photo-only wall pages/progress consumed by later tasks.

- [ ] **Step 1: Add failing catalogue tests for invisible videos**

Add fixtures containing two JPEGs around one MP4 in provisional and captured order. Assert literal page contents and cursors:

```rust
#[test]
fn wall_pages_skip_videos_before_limit_and_cursor_calculation() {
    let fixture = WallFixture::with_assets([
        asset("a.jpg", MediaKind::Jpeg, 1),
        asset("clip.mp4", MediaKind::Video, 2),
        asset("b.jpg", MediaKind::Jpeg, 3),
    ]);

    let first = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, None, 1)
        .unwrap();
    assert_eq!(display_paths(&first.items), ["a.jpg"]);

    let second = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, first.next, 1)
        .unwrap();
    assert_eq!(display_paths(&second.items), ["b.jpg"]);
    assert!(second.next.is_some());
}

#[test]
fn wall_records_for_assets_omit_video_ids() {
    let fixture = WallFixture::with_photo_and_video();
    let rows = fixture
        .catalog
        .wall_records_for_assets(fixture.group, &[fixture.photo, fixture.video])
        .unwrap();
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [fixture.photo]);
}
```

Add an app-service test proving an all-video folder returns `items.is_empty()`, `next_cursor.is_none()`, and the ordinary `No photos found` state through the interface fixture without a hidden-video count.

- [ ] **Step 2: Run the photo-projection tests and verify RED**

Run:

```bash
cargo test -p photo-catalog --test wall_query wall_pages_skip_videos_before_limit_and_cursor_calculation -- --exact
cargo test -p photo-catalog --test wall_query wall_records_for_assets_omit_video_ids -- --exact
cargo test -p photo-app-service --test progressive_wall video_only_selection_has_an_empty_photo_wall -- --exact
```

Expected: FAIL because current wall queries return `media_kind = 'video'` rows and include them in pagination.

- [ ] **Step 3: Add failing migration and repository tests for terminal outcomes**

The migration creates one current terminal outcome per asset and derivative kind, keyed by the immutable derivative key:

```sql
CREATE TABLE derivative_failures (
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK(kind IN ('wall_thumbnail', 'screen_preview')),
  cache_key TEXT NOT NULL,
  availability TEXT NOT NULL,
  failure_code TEXT NOT NULL,
  occurred_at INTEGER NOT NULL,
  PRIMARY KEY(asset_id, kind)
);

CREATE INDEX assets_group_provisional_photo
  ON assets(folder_group_id, provisional_order, id)
  WHERE media_kind <> 'video'
    AND shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL;

CREATE INDEX assets_group_capture_photo
  ON assets(folder_group_id, captured_at_utc, display_path, id)
  WHERE media_kind <> 'video'
    AND shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL
    AND captured_at_utc IS NOT NULL;

CREATE INDEX assets_group_capture_desc_photo
  ON assets(folder_group_id, captured_at_utc DESC, display_path, id)
  WHERE media_kind <> 'video'
    AND shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL
    AND captured_at_utc IS NOT NULL;

PRAGMA user_version = 7;
```

Add round-trip tests for this exact API:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalDerivativeFailure {
    pub asset_id: AssetId,
    pub kind: String,
    pub cache_key: String,
    pub availability: Availability,
    pub failure_code: String,
    pub occurred_at: i64,
}

impl Catalog {
    pub fn find_terminal_derivative_failure(
        &self,
        asset_id: AssetId,
        kind: &str,
        cache_key: &str,
        availability: Availability,
    ) -> Result<Option<TerminalDerivativeFailure>, CatalogError>;

    pub fn record_terminal_derivative_failure(
        &mut self,
        failure: &TerminalDerivativeFailure,
    ) -> Result<(), CatalogError>;

    pub fn clear_terminal_derivative_failure(
        &mut self,
        asset_id: AssetId,
        kind: &str,
    ) -> Result<(), CatalogError>;
}
```

Prove a changed cache key does not match the old failure, a changed availability state does not match it, a new failure replaces the previous key/state, success clears it, and deleting the asset cascades the row. This makes an offline failure terminal only while the asset remains offline; returning online makes it eligible without requiring media bytes to change.

- [ ] **Step 4: Run terminal-outcome tests and verify RED**

Run:

```bash
cargo test -p photo-catalog --test catalog_round_trip terminal_derivative_failure_round_trips_by_current_key -- --exact
cargo test -p photo-catalog --test catalog_round_trip terminal_derivative_failure_is_replaced_cleared_and_cascaded -- --exact
```

Expected: FAIL because migration 7 and the repository API do not exist.

- [ ] **Step 5: Implement photo-only catalogue queries and terminal outcomes**

Add `AND media_kind <> 'video'` before cursor clauses in `wall_page` and in `wall_records_for_assets`. Add a bounded ID page for coordinator traversal:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhotoAssetIdPage {
    pub items: Vec<AssetId>,
    pub next: Option<WallCursorKey>,
}

impl Catalog {
    pub fn photo_asset_ids_page(
        &self,
        group: FolderGroupId,
        order: WallOrder,
        cursor: Option<WallCursorKey>,
        limit: u32,
    ) -> Result<PhotoAssetIdPage, CatalogError> {
        let page = self.wall_page(group, order, cursor, limit)?;
        Ok(PhotoAssetIdPage {
            items: page.items.into_iter().map(|row| row.id).collect(),
            next: page.next,
        })
    }
}
```

Implement migration 7 and `derivative_failure_repo.rs`; export the record from `catalog::lib`.

Update wall query-plan tests to include `media_kind <> 'video'` and assert the new partial indexes are selected.

- [ ] **Step 6: Make scan progress count photos without skipping video indexing**

In each scanner worker, keep emitting discovery, shape, and metadata events for videos, but increment the user-visible progress atomics only for non-video assets:

```rust
let counts_as_photo = item.asset.media_kind != MediaKind::Video;
if counts_as_photo {
    let discovered = progress.0.fetch_add(1, Ordering::Relaxed) + 1;
    publish_discovery_progress(discovered, &progress, &events).await;
}
```

Apply the same condition to shaped and enriched counters. At scan completion set `ScanProgress.total` from the photo-discovered counter, not `ScanSummary.discovered`. Preserve `ScanSummary` as the internal all-media summary.

Add a scanner test with two JPEGs and one MP4 that asserts all three `Discovered` events exist while final progress is exactly `discovered=2`, `shaped=2`, `enriched=2`, `total=Some(2)`.

- [ ] **Step 7: Run Task 1 focused and regression tests**

Run:

```bash
cargo test -p photo-catalog --test wall_query
cargo test -p photo-catalog --test catalog_round_trip
cargo test -p photo-indexer --test progressive_scan
cargo test -p photo-app-service --test progressive_wall video
cargo clippy -p photo-catalog -p photo-indexer -p photo-app-service --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: all pass. Verify fixture MP4 bytes and modification time are unchanged.

- [ ] **Step 8: Commit Task 1**

```bash
git add crates/catalog crates/indexer crates/app-service/tests/progressive_wall.rs
git commit -m "feat: hide indexed videos from photo surfaces"
```

---

### Task 2: Introduce the bounded derivative coordinator state machine

**Files:**
- Create: `crates/app-service/src/derivative_coordinator.rs`
- Modify: `crates/app-service/src/lib.rs`
- Modify: `crates/app-service/src/dto.rs`
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/indexer/src/scheduler.rs`
- Test: `crates/indexer/tests/scheduler_priority.rs`
- Test: unit module in `crates/app-service/src/derivative_coordinator.rs`

**Interfaces:**
- Consumes: `SelectionToken`, `AssetId`, `DerivativeClass`, immutable cache keys, `IndexScheduler`, and terminal outcomes from Task 1.
- Produces: `DerivativeCoordinator`, `WorkKey`, `WorkLane`, `WorkTicket`, `CommitPermit`, `WorkResultReceiver`, and bounded collection state consumed by Tasks 3–5.

- [ ] **Step 1: Add failing scheduler-priority tests**

Extend the priority enum and assert literal dequeue order:

```rust
#[test]
fn derivative_lanes_dequeue_in_product_priority_order() {
    assert_eq!(
        dequeue_names([
            job("idle-preview", JobPriority::IdleLibrary),
            job("idle-wall", JobPriority::OpenCollection),
            job("near-wall", JobPriority::NearViewport),
            job("viewer", JobPriority::ViewerPreview),
            job("visible-wall", JobPriority::Visible),
        ]),
        ["visible-wall", "viewer", "near-wall", "idle-wall", "idle-preview"]
    );
}
```

Add `ViewerPreview` between `NearViewport` and `Visible`; retain existing numeric ordering for lower lanes by using explicit discriminants:

```rust
pub enum JobPriority {
    IdleLibrary = 0,
    OpenCollection = 1,
    NearViewport = 2,
    ViewerPreview = 3,
    Visible = 4,
}
```

- [ ] **Step 2: Run scheduler test and verify RED**

Run: `cargo test -p photo-indexer --test scheduler_priority derivative_lanes_dequeue_in_product_priority_order -- --exact`

Expected: FAIL because `ViewerPreview` does not exist and visible wall work cannot be distinguished from an explicit viewer preview.

- [ ] **Step 3: Add failing coordinator tests for bounded state and promotion**

Define these public-to-crate types in the test before implementing methods:

```rust
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum WorkLane {
    IdlePreview,
    IdleWall,
    NearWall,
    ViewerPreview,
    VisibleWall,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkKey {
    pub(crate) selection: SelectionToken,
    pub(crate) asset_id: AssetId,
    pub(crate) class: DerivativeClass,
    pub(crate) cache_key: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkTicket {
    pub(crate) job_id: u64,
    pub(crate) attempt: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommitPermit {
    pub(crate) job_id: u64,
    pub(crate) attempt: u64,
    pub(crate) selection: SelectionToken,
}

pub(crate) type WorkResultReceiver =
    tokio::sync::oneshot::Receiver<Option<DerivativeReference>>;
```

Add literal tests proving:

```rust
#[tokio::test]
async fn recent_window_is_deduplicated_and_capped_at_250() {
    let coordinator = fixture().coordinator;
    for id in fixture_ids(0..300) {
        coordinator.note_recent(id).await;
    }
    coordinator.note_recent(fixture_id(299)).await;
    assert_eq!(coordinator.recent_ids().await.len(), 250);
    assert_eq!(coordinator.recent_ids().await.first(), Some(&fixture_id(50)));
    assert_eq!(coordinator.recent_ids().await.last(), Some(&fixture_id(299)));
}

#[tokio::test]
async fn foreground_request_promotes_current_background_job_and_waiter() {
    let fixture = fixture();
    let background = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
    let ticket = fixture.coordinator.next_work().await.unwrap();
    let foreground = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;
    assert_eq!(fixture.coordinator.lane(ticket).await, WorkLane::ViewerPreview);
    fixture.coordinator.complete(ticket, reference()).await;
    assert_eq!(background.await.unwrap(), Some(reference()));
    assert_eq!(foreground.await.unwrap(), Some(reference()));
}

#[tokio::test]
async fn invalidated_background_result_cannot_complete_foreground_replacement() {
    let fixture = fixture();
    let stale = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
    let stale_ticket = fixture.coordinator.next_work().await.unwrap();
    fixture.coordinator.invalidate_background().await;
    let foreground = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;
    fixture.coordinator.discard(stale_ticket).await;
    let replacement = fixture.coordinator.next_work().await.unwrap();
    assert_ne!(replacement.attempt, stale_ticket.attempt);
    fixture.coordinator.complete(replacement, reference()).await;
    assert_eq!(stale.await.unwrap(), None);
    assert_eq!(foreground.await.unwrap(), Some(reference()));
}
```

Add tests for selection reset, duplicate-key coalescing, different fingerprint non-coalescing, terminal-state exclusion, collection cursor transitions, and visible promotion over near work.

- [ ] **Step 4: Run coordinator tests and verify RED**

Run:

```bash
cargo test -p photo-app-service derivative_coordinator::tests --lib
```

Expected: FAIL because the coordinator module and its types do not exist.

- [ ] **Step 5: Implement the pure coordinator**

Create one async state authority:

```rust
pub(crate) const RECENT_CAPACITY: usize = 250;

pub(crate) struct DerivativeCoordinator {
    state: tokio::sync::Mutex<CoordinatorState>,
    scheduler: Arc<IndexScheduler>,
    wake: Notify,
}

struct CoordinatorState {
    selection: Option<SelectionToken>,
    background_generation: u64,
    next_job_id: u64,
    next_attempt: u64,
    recent: VecDeque<AssetId>,
    jobs: HashMap<WorkKey, JobState>,
    job_names: HashMap<String, WorkKey>,
    collection: CollectionState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CollectionPhase {
    Dormant,
    Thumbnails,
    Previews,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CollectionState {
    phase: CollectionPhase,
    cursor: Option<WallCursorKey>,
}
```

`enqueue(&self, key: WorkKey, lane: WorkLane) -> WorkResultReceiver` stores separate background and foreground waiters. Promotion changes the lane and re-enqueues the same scheduler name at the higher `JobPriority`. Every issued `WorkTicket` includes a unique attempt; completion and discard ignore mismatched attempts.

Map lanes to the shared scheduler without relying on enum casts:

```rust
fn scheduler_priority(lane: WorkLane) -> JobPriority {
    match lane {
        WorkLane::IdlePreview => JobPriority::IdleLibrary,
        WorkLane::IdleWall => JobPriority::OpenCollection,
        WorkLane::NearWall => JobPriority::NearViewport,
        WorkLane::ViewerPreview => JobPriority::ViewerPreview,
        WorkLane::VisibleWall => JobPriority::Visible,
    }
}
```

`invalidate_background` increments the background generation, discards queued background-only jobs, marks running background-only attempts stale, and waits for already-admitted background commits before returning. `admit_commit` atomically moves a current running attempt to `Committing` and returns `CommitPermit`; later invalidation treats that permit as already ordered.

Add `Hash` derives to `DerivativeClass` and `SelectionToken` so `WorkKey` has a real value identity rather than a formatted-string surrogate.

Do not put filesystem or catalogue calls in this module.

- [ ] **Step 6: Run Task 2 tests**

Run:

```bash
cargo test -p photo-indexer --test scheduler_priority
cargo test -p photo-app-service derivative_coordinator::tests --lib
cargo clippy -p photo-indexer -p photo-app-service --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: all pass. Include a mutation check: removing the 250-item pop-front must fail the bounded-window test; accepting a mismatched attempt must fail the stale replacement test.

- [ ] **Step 7: Commit Task 2**

```bash
git add crates/indexer/src/scheduler.rs crates/indexer/tests/scheduler_priority.rs crates/app-service/src/derivative_coordinator.rs crates/app-service/src/lib.rs crates/app-service/src/dto.rs crates/app-service/src/service.rs
git commit -m "refactor: add bounded derivative coordinator"
```

---

### Task 3: Route foreground derivative requests through the coordinator

**Files:**
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/derivative_coordinator.rs`
- Test: `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**
- Consumes: `DerivativeCoordinator::enqueue`, `next_work`, `invalidate_background`, promotion semantics, terminal failure repository, and existing `DerivativeRequest` DTOs.
- Produces: coordinator-owned visible wall and explicit viewer-preview requests, prerequisite thumbnail repair, and deterministic foreground results used by the interface.

- [ ] **Step 1: Add failing tests for request ordering and stale-background collision**

Add integration tests using real `AppService`, cache, catalogue, and deterministic worker gates:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_preview_generates_and_publishes_thumbnail_before_preview() {
    let fixture = Fixture::one_jpeg_without_derivatives();
    let mut updates = fixture.service.watch_updates();

    fixture.service.request_derivatives(
        DerivativeRequest::visible_screen_preview(vec![fixture.asset_id_string()]),
    ).await.unwrap();

    assert_eq!(next_derivative_kind(&mut updates).await, DerivativeClass::WallThumbnail);
    assert_eq!(next_derivative_kind(&mut updates).await, DerivativeClass::ScreenPreview);
    assert_eq!(fixture.catalog_kinds(), ["wall_thumbnail", "screen_preview"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn viewer_request_restarts_once_after_colliding_background_was_invalidated() {
    let fixture = Fixture::one_jpeg_with_wall_thumbnail();
    let background = fixture.begin_blocked_background_preview().await;
    fixture.service.request_derivatives(DerivativeRequest::visible(vec![fixture.asset_id_string()])).await.unwrap();
    let foreground = fixture.request_visible_preview();
    fixture.release(background).await;
    assert!(foreground.await.is_ok());
    assert_eq!(fixture.screen_encode_count(), 2);
    assert_eq!(fixture.screen_commit_count(), 1);
}
```

Add a promotion test where the background job is still current: foreground joins it, encode count is 1, commit count is 1, and both waiters succeed.

Add `direct_video_derivative_request_is_rejected_without_work`: pass an indexed MP4 ID to both wall-thumbnail and screen-preview requests, expect `DerivativeUnavailable`, and assert zero coordinator jobs, source reads, cache files, catalogue derivatives, and publications.

- [ ] **Step 2: Run Task 3 tests and verify RED**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall explicit_preview_generates_and_publishes_thumbnail_before_preview -- --exact
cargo test -p photo-app-service --test progressive_wall viewer_request_restarts_once_after_colliding_background_was_invalidated -- --exact
cargo test -p photo-app-service --test progressive_wall viewer_request_promotes_current_background_encode -- --exact
cargo test -p photo-app-service --test progressive_wall direct_video_derivative_request_is_rejected_without_work -- --exact
```

Expected: at least the collision test FAILS because the existing same-key queue joins foreground waiters to stale background generation.

- [ ] **Step 3: Replace the old request queue and preview-gate ownership**

Change `AppService` to own:

```rust
pub(crate) coordinator: Arc<DerivativeCoordinator>,
```

Remove `DerivativeQueue`, `PreviewGateState`, `preview_gate_wake`, `screen_preview_commit_lock`, recent IDs from `ServiceState`, and their independent generation counters after callers migrate. Do not retain a second job map or cancellation generation in `derivatives.rs`.

Map request lanes exactly:

```rust
fn request_lane(request: &DerivativeRequest) -> WorkLane {
    match (request.kind, request.priority) {
        (DerivativeClass::WallThumbnail, DerivativePriority::Visible) => WorkLane::VisibleWall,
        (DerivativeClass::WallThumbnail, DerivativePriority::NearViewport) => WorkLane::NearWall,
        (DerivativeClass::ScreenPreview, DerivativePriority::Visible) => WorkLane::ViewerPreview,
        (DerivativeClass::ScreenPreview, DerivativePriority::NearViewport) => WorkLane::IdlePreview,
    }
}
```

A `ViewerPreview` request first resolves or enqueues that asset's wall thumbnail. It enqueues the screen preview only after the wall result succeeds or a matching ready catalogue record already exists. A terminal wall failure returns `DerivativeUnavailable` without queuing screen work.

Before key resolution, reject any asset whose catalogue `media_kind` is `Video`. Do not add it to recent state or emit an asset warning; videos are intentionally outside the photo pipeline rather than failed photos.

Visible wall requests call `coordinator.invalidate_background()` through the coordinator state boundary before enqueueing, so queued idle previews yield immediately. A viewer request promotes compatible current background work instead of invalidating it.

- [ ] **Step 4: Preserve local legacy repair under coordinator ownership**

Retain `ImageDerivativeGenerator::generate_wall_thumbnail_from_cached_preview`, but resolve the cached preview path only through `CacheWriter::resolve_checked` and only when its derivative record matches asset, group, and current screen key. The wall job still owns catalogue registration and publication.

Add an offline test that removes the fixture source after a screen preview exists, requests the wall thumbnail, and asserts:

```rust
assert_eq!(published_kinds, [DerivativeClass::WallThumbnail]);
assert_eq!(source_read_count, 0);
assert_eq!(source_hash_before, source_hash_after_restore);
assert_eq!(source_mtime_before, source_mtime_after_restore);
```

- [ ] **Step 5: Run Task 3 focused and regression tests**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall explicit_preview
cargo test -p photo-app-service --test progressive_wall viewer_request
cargo test -p photo-app-service --test progressive_wall legacy_screen_preview
cargo test -p photo-cache --test image_derivative repairs_a_wall_thumbnail_from_a_cached_screen_preview -- --exact
cargo clippy -p photo-app-service -p photo-cache --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: all pass. Confirm no production source mutation token was introduced with the existing source-safety audit command from the zoom/pan verification document.

- [ ] **Step 6: Commit Task 3**

```bash
git add crates/app-service crates/cache
git commit -m "refactor: coordinate foreground derivative requests"
```

---

### Task 4: Make screen-preview commit admission linearizable

**Files:**
- Modify: `crates/app-service/src/derivative_coordinator.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/service.rs`
- Test: `crates/app-service/tests/progressive_wall.rs`
- Test: `crates/cache/tests/cache_budget.rs`

**Interfaces:**
- Consumes: `WorkTicket`, `DerivativeCoordinator::admit_commit`, and cache-budget/catalogue APIs.
- Produces: one `CommitPermit` linearization point covering background invalidation, cache eviction/write, catalogue registration, warning convergence, publication, and waiter completion.

- [ ] **Step 1: Add a failing deterministic time-of-check/time-of-use race**

Install a test-only gate after screen encoding but before `admit_commit`, and a second gate after permit admission but before the managed-cache commit. Seed one evictable screen preview and one warning so every prohibited side effect is observable:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalidation_before_commit_admission_has_no_side_effects() {
    let fixture = Fixture::with_wall_ready_asset_and_evictable_preview();
    fixture.seed_derivative_warning();
    let before = fixture.snapshot_cache_catalog_warnings();

    let blocked = fixture.begin_screen_encode_blocked_before_admission().await;
    fixture.invalidate_with_visible_wall_request().await;
    fixture.release(blocked).await;
    fixture.await_worker_completion().await;

    assert_eq!(fixture.snapshot_cache_catalog_warnings(), before);
    assert_eq!(fixture.screen_publications(), []);
    assert_eq!(fixture.pending_worker_count(), 0);
}
```

Add the complementary ordering test:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_commit_finishes_before_later_invalidation() {
    let fixture = Fixture::with_wall_ready_asset();
    let admitted = fixture.begin_screen_commit_blocked_after_admission().await;
    let visible_request = fixture.start_visible_wall_invalidation();
    assert!(!visible_request.is_finished());
    fixture.release(admitted).await;
    visible_request.await.unwrap();
    assert_eq!(fixture.screen_derivative_count(), 1);
    assert_eq!(fixture.screen_publication_count(), 1);
}
```

The second test proves the defined linearization rule: once admitted, the commit is ordered before the later visible request rather than retroactively becoming stale.

- [ ] **Step 2: Run race tests and verify RED**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall invalidation_before_commit_admission_has_no_side_effects -- --exact
cargo test -p photo-app-service --test progressive_wall admitted_commit_finishes_before_later_invalidation -- --exact
```

Expected: FAIL because current code validates generation outside a shared state transition and visible invalidation can race between the check and cache/catalogue effects.

- [ ] **Step 3: Implement permit admission and completion**

The coordinator API is:

```rust
impl DerivativeCoordinator {
    pub(crate) async fn admit_commit(
        &self,
        ticket: WorkTicket,
        selection: SelectionToken,
        prerequisite_key: Option<&str>,
    ) -> Option<CommitPermit>;

    pub(crate) async fn complete_commit(
        &self,
        permit: CommitPermit,
        reference: DerivativeReference,
    ) -> Vec<oneshot::Sender<Option<DerivativeReference>>>;

    pub(crate) async fn fail_commit(
        &self,
        permit: CommitPermit,
    ) -> Vec<oneshot::Sender<Option<DerivativeReference>>>;
}
```

`admit_commit` verifies the exact attempt, active selection, immutable key, current foreground/background status, and matching wall prerequisite. It changes the job to `Committing` while holding the coordinator state mutex. `invalidate_background` never invalidates `Committing`; it waits on that job's completion notification before returning. Therefore an admitted job is both logically and observably ordered before the later visible request.

After encoding, call `admit_commit` before any of these operations:

- cache-budget authorization or eviction planning;
- cache file creation or rename;
- derivative catalogue insert/upsert;
- terminal-failure clearing;
- warning clearing;
- `WallUpdate::DerivativesReady` publication.

If admission returns `None`, drop the encoded value and finish the stale attempt without warnings. If admitted, perform the contained cache and catalogue commit, converge warnings, publish the immutable outcome, then call `complete_commit`. On commit error, record the correct asset/cache warning and call `fail_commit`.

Do not hold the coordinator state mutex while encoding source bytes. Test-only gates remain behind `cfg(any(test, debug_assertions))` and expose no production path or source filename.

- [ ] **Step 4: Assert real eviction and warning behavior**

Extend the race fixture so the cache budget would evict a known old folder group if commit were admitted. Assert the old derivative remains before admission and is removed only in the complementary admitted test. Assert an asset warning is unchanged when stale work is discarded and cleared only by an admitted successful commit.

- [ ] **Step 5: Run Task 4 tests**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall invalidation_before_commit
cargo test -p photo-app-service --test progressive_wall admitted_commit
cargo test -p photo-cache --test cache_budget
cargo test -p photo-cache --test cache_policy
cargo clippy -p photo-app-service -p photo-cache --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: all pass. Run each race test at least 20 times in one test invocation or a literal loop inside the test so scheduler timing cannot hide the boundary failure.

- [ ] **Step 6: Commit Task 4**

```bash
git add crates/app-service crates/cache/tests/cache_budget.rs
git commit -m "fix: linearize preview commit admission"
```

---

### Task 5: Run bounded two-phase collection work without starvation

**Files:**
- Modify: `crates/app-service/src/derivative_coordinator.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/scan.rs`
- Modify: `crates/app-service/src/service.rs`
- Test: `crates/app-service/tests/progressive_wall.rs`
- Test: `crates/catalog/tests/wall_query.rs`

**Interfaces:**
- Consumes: photo-only `Catalog::photo_asset_ids_page`, terminal outcomes, coordinator collection cursors, worker lanes, interaction mode, and linearizable commits.
- Produces: thumbnail phase followed by preview phase, bounded at 250 IDs per page and resilient to corrupt, unsupported, unavailable, or terminal assets.

- [ ] **Step 1: Add a failing multi-page phase-order test**

Use 301 JPEG fixtures with a gate on asset 300's wall job. Start idle collection work and assert no screen encode, cache row, or publication occurs while that late wall is blocked:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_photo_page_reaches_wall_outcome_before_background_screens_begin() {
    let fixture = Fixture::jpeg_collection(301);
    let late_wall = fixture.block_wall_asset(300).await;
    fixture.start_idle_collection().await;
    fixture.await_wall_attempt(300).await;

    assert_eq!(fixture.screen_encode_count(), 0);
    assert_eq!(fixture.screen_derivative_count(), 0);
    assert_eq!(fixture.screen_publication_count(), 0);

    fixture.release(late_wall).await;
    fixture.await_first_screen_publication().await;
    assert_eq!(fixture.wall_ready_count(), 301);
}
```

- [ ] **Step 2: Add failing non-starvation and video tests**

Create a collection containing valid JPEG, corrupt JPEG, unavailable JPEG, valid JPEG, and MP4. Assert exact outcomes:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_photo_failures_and_videos_do_not_starve_healthy_previews() {
    let fixture = Fixture::mixed_collection();
    fixture.run_idle_collection_to_completion().await;

    assert_eq!(fixture.wall_ready_names(), ["a.jpg", "b.jpg"]);
    assert_eq!(fixture.terminal_wall_names(), ["corrupt.jpg", "offline.jpg"]);
    assert_eq!(fixture.screen_ready_names(), ["a.jpg", "b.jpg"]);
    assert_eq!(fixture.derivative_names_for("clip.mp4"), []);
    assert_eq!(fixture.photo_page_names(), ["a.jpg", "corrupt.jpg", "offline.jpg", "b.jpg"]);
    assert_eq!(fixture.coordinator_phase(), CollectionPhase::Complete);
}
```

Restart the service with the same catalogue and prove terminal failures with the same key and availability are not attempted again. Change the corrupt asset's modified timestamp/key and prove one new attempt occurs. Mark the unavailable asset available without changing its media signature and prove the availability mismatch permits one new attempt.

- [ ] **Step 3: Run phase and starvation tests and verify RED**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall every_photo_page_reaches_wall_outcome_before_background_screens_begin -- --exact
cargo test -p photo-app-service --test progressive_wall terminal_photo_failures_and_videos_do_not_starve_healthy_previews -- --exact
cargo test -p photo-app-service --test progressive_wall terminal_failure_retries_only_after_derivative_key_changes -- --exact
```

Expected: FAIL because the current implementation either interleaves preview pages, blocks on a failed wall count, retries failures after wakes, or admits video candidates.

- [ ] **Step 4: Implement the thumbnail phase**

When scanning settles or interaction becomes idle, call `coordinator.begin_collection(selection)`. The driver repeatedly:

1. reads at most 250 photo IDs from `photo_asset_ids_page`;
2. resolves current wall keys and ready catalogue records;
3. skips matching terminal outcome rows;
4. enqueues missing work as `IdleWall`;
5. awaits all page results;
6. records terminal asset failures using the current wall key;
7. advances the thumbnail cursor only after the page reaches ready or terminal state;
8. pauses before the next page when interaction is active or selection changes.

At end of the final page, atomically transition collection state from `Thumbnails` to `Previews { cursor: None }`. Do not retain page IDs after advancing and never copy every asset into `recent`.

Cache-wide failures return `Blocked` and bounded retry/backoff; they do not become terminal asset failures.

For a terminal asset failure, upsert the matching `derivative_failures` row and converge the public warning to code `derivativeUnavailable` with `retryable: false`. Update `map_asset_warning_code` so the persisted terminal warning code `derivative_generation_terminal` maps to that non-retryable DTO, while transient `derivative_generation_failed` remains retryable. A successful derivative clears both the terminal row and its terminal warning. A changed derivative key remains eligible even while the old terminal row exists; the new attempt replaces or clears it.

- [ ] **Step 5: Implement the preview phase**

The preview driver starts only when `CollectionState.phase == CollectionPhase::Previews`. It traverses the same photo-only pages, selects assets with matching ready wall thumbnails and without matching terminal screen failures, and enqueues at `IdlePreview`.

Process the bounded recent window first by reading at most 250 IDs and enqueueing `ViewerPreview` only for an active explicit request; ordinary recent background work remains `IdlePreview`. Clear the recent window after consumption. Then process collection pages at `IdlePreview`.

An active interaction pauses `IdleWall` and `IdlePreview` between jobs/pages. `VisibleWall`, `ViewerPreview`, and `NearWall` continue with the scheduler's active worker limit. A new visible wall request calls `invalidate_background`, returns collection state to the correct thumbnail cursor, and prevents a background screen page from being admitted until thumbnail work drains again.

- [ ] **Step 6: Add boundedness assertions**

Expose counters only under `cfg(debug_assertions)`:

```rust
pub struct CoordinatorTestSnapshot {
    pub recent_len: usize,
    pub largest_loaded_page: usize,
    pub queued_jobs: usize,
    pub phase: CollectionPhase,
}
```

In a 10,000-asset fixture, assert `recent_len <= 250`, `largest_loaded_page <= 250`, and queued jobs never exceed 250 plus active foreground requests. Do not expose IDs or paths.

- [ ] **Step 7: Run Task 5 tests**

Run:

```bash
cargo test -p photo-app-service --test progressive_wall collection
cargo test -p photo-app-service --test progressive_wall terminal
cargo test -p photo-app-service --test progressive_wall video
cargo test -p photo-app-service --test progressive_wall interaction
cargo test -p photo-catalog --test wall_query
cargo clippy -p photo-app-service -p photo-catalog --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: all pass with no timeout, unbounded allocation, video derivative, or repeated terminal attempt.

- [ ] **Step 8: Commit Task 5**

```bash
git add crates/app-service crates/catalog/tests/wall_query.rs
git commit -m "feat: run thumbnail-first collection phases"
```

---

### Task 6: Make thumbnail paint the wall before enabling the viewer

**Files:**
- Modify: `apps/interface/src/components/PhotoTile.tsx`
- Modify: `apps/interface/src/styles/photoWall.module.css`
- Modify: `apps/interface/src/components/PhotoWall.browser.test.tsx`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/viewer/useViewerPreview.ts`

**Interfaces:**
- Consumes: photo-only `WallAsset[]`, wall-thumbnail derivative references, coordinator-backed `requestDerivatives`, existing viewer transform and controls.
- Produces: stable paint phases, a separate interactive overlay, explicit preview requests only after opening a painted tile, layered Escape, and discrete live announcements.

- [ ] **Step 1: Add failing browser tests for paint-before-open**

Use a real image URL under the WebKit fixture and intercept its readiness callbacks. Assert the button does not exist until fade completion:

```tsx
it("keeps the current tile inert through decode and fade, then enables it", async () => {
  const fixture = renderWallWithControlledThumbnail();
  fixture.publishWallThumbnail("photo-a");

  expect(screen.queryByRole("button", { name: "Open Photo A" })).toBeNull();
  fixture.releaseImageLoad("photo-a");
  await fixture.releaseDecode("photo-a");
  expect(screen.queryByRole("button", { name: "Open Photo A" })).toBeNull();

  fixture.finishOpacityTransition("photo-a");
  expect(screen.getByRole("button", { name: "Open Photo A" })).toBeVisible();
  expect(fixture.image("photo-a")).toHaveStyle({ opacity: "1" });
});
```

Add separate tests for image error, URL replacement during decode, stale transition end, cached-complete image, and reduced motion. Every failure/stale case must have no open button and no `onOpen` call.

- [ ] **Step 2: Run wall browser tests and verify RED**

Run: `npm run test:browser --workspace @photo-viewer/interface -- PhotoWall.browser.test.tsx`

Expected: FAIL because current readiness toggles the root between `figure` and `button`, remounts the image, and exposes interaction before the fade is proven complete.

- [ ] **Step 3: Implement a stable tile paint state machine**

Keep one stable `<figure>` root, image, placeholder layers, and warning layers. Add a separate absolute button only in the `interactive` phase:

```ts
type TilePaintPhase =
  | "placeholder"
  | "decoding"
  | "fading"
  | "interactive"
  | "failed";

interface TileRevision {
  assetId: string;
  derivativeKey: string;
  url: string;
}
```

On current URL change, increment a local revision and return to `decoding`. `onLoad` plus successful `decode()` moves only the matching revision to `fading`. `onTransitionEnd` for `propertyName === "opacity"` moves only the matching revision to `interactive`. Under `prefers-reduced-motion: reduce`, schedule one `requestAnimationFrame` after decode and then move to `interactive`; cancel it on revision change/unmount.

Render:

```tsx
<figure className={styles.tile} data-asset-id={asset.id}>
  {layers}
  {phase === "interactive" ? (
    <button
      aria-label={`Open ${asset.displayName}`}
      className={styles.tileOpenOverlay}
      onClick={() => onOpen(asset.id)}
      type="button"
    />
  ) : null}
</figure>
```

The overlay has transparent background, fills the tile, inherits focus styling, and does not cover the warning icon visually. Video branches are removed from `PhotoTile` because video DTOs no longer reach the interface; retain defensive inert rendering for an unexpected `mediaKind === "video"` fixture without a caption or count.

- [ ] **Step 4: Add failing viewer tests for request and Escape lifecycle**

Assert opening a painted tile causes one visible screen-preview request for the current asset. Assert a screen-only colour block cannot open and therefore makes no preview request.

Retain and strengthen the combined Escape test:

```tsx
it("unwinds drawer, zoom, then viewer across three Escape presses", async () => {
  const fixture = await openZoomedViewerWithDrawer();
  await press("Escape");
  expect(fixture.drawer()).not.toBeInTheDocument();
  expect(fixture.viewer()).toBeVisible();
  expect(fixture.zoomLabel()).not.toHaveTextContent("Fit");

  await press("Escape");
  expect(fixture.viewer()).toBeVisible();
  expect(fixture.zoomLabel()).toHaveTextContent("Fit");

  await press("Escape");
  expect(fixture.viewer()).not.toBeInTheDocument();
  expect(fixture.wall()).toBeVisible();
});
```

Use `MutationObserver` to prove the second Escape creates a discrete live-region mutation even if the prior announced text was `Fit`; wheel/pinch frames must create no live-region mutation.

- [ ] **Step 5: Run viewer browser tests and verify RED for missing behavior only**

Run:

```bash
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
```

Expected: the paint/request test FAILS before the stable overlay implementation. The existing Escape and live-region regressions must remain green at this checkpoint; no production change is required for behavior they already protect.

- [ ] **Step 6: Implement viewer request and retain dark appearance**

`useViewerPreview` requests the explicit current screen preview only after the viewer opens from an interactive wall tile. It keeps current/neighbor request planning bounded and does not request any video ID.

In the global Escape handler, use current refs in this order:

```ts
if (infoOpenRef.current) {
  onSetInfoOpenRef.current(false);
} else if (transformModeRef.current === "zoomed") {
  discreteResetRef.current();
} else {
  onCloseRef.current();
}
```

Keep the event/revision-based live announcement. Do not subscribe it to continuous transform state. Add a contrast test that renders the viewer under system-light appearance and asserts its computed canvas/chrome background uses the dark viewer token; this documents the deliberate exception without changing theme CSS.

- [ ] **Step 7: Run Task 6 interface gates**

Run:

```bash
npm test
npm run test:browser
npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion
npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast
npm run typecheck
npm run check
npm run --workspace @photo-viewer/interface build
git diff --check
```

Expected: all pass. The existing non-failing React `act(...)` warning must not increase; new tests must await their own asynchronous image work.

- [ ] **Step 8: Commit Task 6**

```bash
git add apps/interface
git commit -m "fix: paint thumbnails before opening photos"
```

---

### Task 7: Verify large-library behavior, source safety, desktop integration, and the native demo

**Files:**
- Modify: `crates/catalog-bench/src/lib.rs`
- Modify: `crates/catalog-bench/tests/benchmark_smoke.rs`
- Modify: `docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md`
- Modify: `README.md`
- Test: existing workspace, desktop, browser, cache, and benchmark suites

**Interfaces:**
- Consumes: the complete coordinator, photo-only projections, stable wall tiles, viewer behavior, and existing desktop profile/bundle workflow.
- Produces: reproducible verification evidence and a running macOS app for user acceptance. No merge or push.

- [ ] **Step 1: Extend the million-asset benchmark with photo/video and coordinator projections**

Seed a deterministic mix of 90 percent stills and 10 percent videos in a shaped benchmark folder group. Record separate timings for:

- first 100-photo wall page with videos interleaved;
- second page cursor continuation;
- 250-ID coordinator photo page;
- terminal-failure lookup for a current key;

Extend `BenchmarkReport` with `second_page_ms`, `second_page_rows`, `coordinator_page_ms`, and `coordinator_page_rows`. Assert returned page sizes are exactly 100/100/250 and no result is a video. Keep bounded recent-window coverage in the Task 2 coordinator test so the benchmark crate does not depend on `photo-app-service`. Preserve the existing insert, unavailable-count, and eviction-plan measurements.

- [ ] **Step 2: Run the benchmark and record fresh numbers**

Run:

```bash
cargo run --release -p catalog-bench -- --assets 1000000 --output /tmp/photo-viewer-million-report.json
cargo test -p catalog-bench --test benchmark_smoke
```

Record compile-warm timings and machine context in the verification document. The acceptance threshold is no worse than 20 percent above the previous first-page measurement and no collection-sized allocation or queue snapshot.

- [ ] **Step 3: Run the complete interface verification matrix**

Run:

```bash
npm test
npm run test:browser
npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion
npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast
npm run typecheck
npm run check
npm run --workspace @photo-viewer/interface build
```

Record exact test counts and bundle asset sizes. Treat any new console warning, unhandled promise rejection, accessibility violation, or screenshot attachment as a failure.

- [ ] **Step 4: Run the complete Rust and desktop verification matrix**

Run:

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check
npm run desktop:build
git diff --check
```

Record exact counts and the absolute `.app` bundle path.

- [ ] **Step 5: Audit source and cache safety**

Hash the controlled source fixture tree before and after a clean-profile scan, legacy cache repair, thumbnail phase, preview phase, sorting, viewer opening, zoom, and app restart. Assert identical source hashes and modification times.

Inspect the feature diff from its accepted base for production source-media write/delete/rename/move/copy calls and native-path DTO leakage. Record the exact base and head SHAs and every broad-token match with its harmless context. Verify managed cache paths remain under the named profile cache root.

- [ ] **Step 6: Update user and verification documentation**

Document:

- videos are indexed but invisible until cross-platform playback ships;
- the wall must paint a thumbnail before a photo opens;
- background order is thumbnails, then previews;
- corrupt photos do not block healthy work;
- Escape closes Info, then resets zoom, then returns to the wall;
- the viewer intentionally stays dark under system-light mode;
- cache-only zoom limitations remain.

Do not claim video playback, original-resolution zoom, or public-web security.

- [ ] **Step 7: Commit automated verification and documentation**

```bash
git add crates/catalog-bench docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md README.md
git commit -m "test: verify coordinated photo loading"
```

- [ ] **Step 8: Launch the macOS acceptance build without merging**

Run from the feature worktree:

```bash
PHOTO_VIEWER_PROFILE=wall-demo npm run desktop:dev
```

Leave the process running and ask the user to demonstrate:

- videos are absent with no hidden count;
- current-viewport thumbnails paint first and tiles do not open early;
- a corrupt asset does not stall healthy photos;
- opening a painted tile refines to its cached larger preview;
- Info → zoom → wall Escape unwinding;
- dark viewer under system-light appearance;
- restart and offline cached behavior.

Native user acceptance is mandatory before branch integration. Do not merge, push, delete the worktree, or alter the user's normal profile.
