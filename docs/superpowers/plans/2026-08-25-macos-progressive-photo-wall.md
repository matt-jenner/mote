# macOS progressive photo wall implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver checkpoint 2 as a running macOS application that fills a vertically scrolling justified wall before an uncached scan finishes, caches wall and screen derivatives, and switches between oldest-first and newest-first catalog order.

**Architecture:** Extend SQLite with stable provisional ordering and wall queries, then split the scanner so shape events commit before metadata and colour work. A host-neutral application service owns scan orchestration, derivative scheduling, and ordered wall updates. The shared React interface consumes that service through bounded queries and a pure justified-row engine; the Tauri adapter supplies typed commands, a channel, and a cache-only custom protocol.

**Tech Stack:** Rust 1.97.1 and edition 2024; SQLite through `photo-catalog`; Tokio 1.53.1; `image` 0.25.10; Tauri 2.11.5; React 19.2.8; TypeScript 7.0.2; Vite 8.2.2; TanStack Query 5.102.2; Vitest 4.1.11 with Playwright WebKit; CSS Modules; Biome 2.5.10.

**Spec:** `docs/superpowers/specs/2026-08-25-macos-progressive-photo-wall-design.md`

## Global constraints

- This plan implements checkpoint 2 only. Row virtualization, continuous wall sizing, exact anchor preservation, and the immersive viewer remain later checkpoints.
- Every production change follows test-driven development. Run the named test and observe the expected failure before writing implementation code.
- Every task ends with focused verification and a logical commit.
- Source media is read-only. Production code may open and read source files but may never write, rename, move, or delete them.
- SQLite stores metadata and derivative references, never image blobs.
- The managed cache is the only writable image location. Cache writes use `CacheWriter::write_atomic` and content-addressed relative paths.
- Wall thumbnails use a 1024-pixel long edge and the durable cache tier.
- Screen previews use a 4096-pixel long edge capped at source dimensions and the non-durable group-evicted tier.
- The default settled order is resolved capture date ascending. The direction control supports oldest-first and newest-first only in this checkpoint.
- An uncached source uses persisted provisional order until its first metadata pass settles. Individual metadata arrivals never reorder visible rows.
- The wall queries SQLite only. Scroll and sort operations never enumerate the source.
- Geometry, catalog, and derivative updates cross the desktop boundary in batches.
- Only `apps/interface/src/services/tauriPhotoService.ts` imports `@tauri-apps/api`.
- React components consume `PhotoService`; they never construct native paths or transport-specific URLs.
- The `photo-derivative:` protocol serves managed cache files only and validates asset ID, derivative kind, immutable key, and cache-root containment.
- Existing metadata precedence, multiple-root identity, offline retention, and cache-group eviction behavior remain intact.
- New motion uses the existing 160 ms token and becomes zero under `prefers-reduced-motion`.
- Shared interface tests cover 1440 by 1024, 834 by 1194, and 390 by 844 CSS pixels.
- The checkpoint finishes with real photographic fixtures in the running macOS application, fresh screenshots, and a short motion capture.

## File map

### Catalog and indexing

- `crates/catalog/migrations/0004_wall_projection.sql`: provisional order, shape state, and wall-query indexes.
- `crates/catalog/src/wall_repo.rs`: bounded provisional or capture-date wall pages and cursor keys.
- `crates/catalog/src/cache_repo.rs`: derivative lookup by asset, kind, and immutable key.
- `crates/indexer/src/scanner.rs`: independent discovery, shape, colour, and metadata stages.
- `crates/indexer/src/events.rs`: shape-ready, colour-ready, progress, and completion events.
- `crates/indexer/src/catalog_writer.rs`: generation-aware transactional event batches.
- `crates/indexer/src/default_reader.rs`: EXIF, sidecar XMP, filesystem birth, and modified-date bundle.

### Derivatives and application service

- `crates/cache/src/image_derivative.rs`: orientation, bounded resize, representative colour, JPEG encoding, and atomic cache output.
- `crates/cache/src/budget.rs`: automatic large-derivative limit and pre-write whole-group eviction.
- `crates/app-service/src/wall.rs`: public wall DTOs, opaque cursors, and catalog mapping.
- `crates/app-service/src/scan.rs`: active-source scan lifecycle and ordered update batching.
- `crates/app-service/src/derivatives.rs`: priority requests and derivative-ready batches.
- `crates/app-service/src/service.rs`: cloneable shared state and checkpoint operations.

### Desktop host

- `apps/desktop/src-tauri/src/commands.rs`: wall query, derivative request, interaction, and update-channel commands.
- `apps/desktop/src-tauri/src/protocol.rs`: read-only `photo-derivative:` handler.
- `apps/desktop/src-tauri/src/state.rs`: cloneable `AppService` state without an outer blocking mutex.
- `apps/desktop/src-tauri/tauri.conf.json`: cache protocol in the image CSP.

### Shared interface

- `apps/interface/src/services/photoService.ts`: host-neutral wall records and operations.
- `apps/interface/src/services/inMemoryPhotoService.ts`: deterministic slow progressive source.
- `apps/interface/src/services/tauriPhotoService.ts`: typed commands, `Channel<WallUpdate>`, and derivative URL mapping.
- `apps/interface/src/wall/layoutJustifiedRows.ts`: pure complete-row geometry.
- `apps/interface/src/wall/wallReducer.ts`: stable batch merge, sorting reset, and settled replacement.
- `apps/interface/src/app/usePhotoWall.ts`: service subscription, bounded loading, and interaction signals.
- `apps/interface/src/components/PhotoWallCanvas.tsx`: wall screen orchestration.
- `apps/interface/src/components/WallToolbar.tsx`: direction and progress controls.
- `apps/interface/src/components/JustifiedWall.tsx`: rows, load sentinel, and tile rendering.
- `apps/interface/src/components/PhotoTile.tsx`: neutral, representative-colour, and derivative layers.
- `apps/interface/src/styles/photoWall.module.css`: wall-only layout and transitions.

---

### Task 1: Add the stable wall projection to SQLite

**Files:**
- Create: `crates/catalog/migrations/0004_wall_projection.sql`
- Create: `crates/catalog/src/wall_repo.rs`
- Create: `crates/catalog/tests/wall_query.rs`
- Modify: `crates/catalog/src/migrate.rs`
- Modify: `crates/catalog/src/lib.rs`
- Modify: `crates/catalog/src/asset_repo.rs`
- Modify: `crates/catalog/src/index_repo.rs`
- Modify: `crates/catalog/src/cache_repo.rs`
- Test: `crates/catalog/tests/catalog_round_trip.rs`

**Interfaces:**
- Consumes: `LibraryId`, `AssetId`, `AssetRecord`, scan generations, and existing derivative rows.
- Produces: `ShapeStatus`, `WallOrder`, `WallCursorKey`, `WallCatalogRecord`, `WallCatalogPage`, `Catalog::wall_page`, `Catalog::has_completed_generation`, `Catalog::derivatives_for_assets`, and `Catalog::find_derivative`.

- [ ] **Step 1: Write failing wall-query tests**

```rust
// crates/catalog/tests/wall_query.rs
use std::path::Path;
use photo_catalog::{Catalog, NewAsset, NewLibrary, ShapeStatus, WallOrder};
use photo_domain::{MediaKind, RelativePathKey};

#[test]
fn new_assets_keep_append_only_provisional_order_across_upserts() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog.add_library(&NewLibrary::configured("Photos", Path::new("/Photos"))).unwrap();
    let group = ready_group(&mut catalog, library.id);
    for path in ["b.jpg", "a.jpg", "c.jpg"] {
        let key = RelativePathKey::from_relative_path(path.as_ref()).unwrap();
        let mut asset = NewAsset::minimal(library.id, key, path, MediaKind::Jpeg, 10);
        asset.folder_group_id = Some(group);
        catalog.upsert_asset(&asset).unwrap();
        catalog.apply_index_batch(&[CatalogIndexRecord::Shaped(AssetShapeUpdate {
            asset_id: asset.id,
            width: 16,
            height: 9,
            orientation: Some(1),
            representative_rgb: None,
            shape_status: ShapeStatus::Ready,
        })]).unwrap();
    }
    let first = catalog.wall_page(group, WallOrder::Provisional, None, 10).unwrap();
    assert_eq!(first.items.iter().map(|item| item.display_path.as_str()).collect::<Vec<_>>(), ["b.jpg", "a.jpg", "c.jpg"]);

    let key = RelativePathKey::from_relative_path("b.jpg".as_ref()).unwrap();
    let mut changed = NewAsset::minimal(library.id, key, "b.jpg", MediaKind::Jpeg, 11);
    changed.folder_group_id = Some(group);
    catalog.upsert_asset(&changed).unwrap();
    let second = catalog.wall_page(group, WallOrder::Provisional, None, 10).unwrap();
    assert_eq!(second.items.iter().map(|item| item.provisional_order).collect::<Vec<_>>(), [1, 2, 3]);
}

#[test]
fn settled_pages_sort_dates_in_both_directions_with_stable_ties() {
    let mut fixture = WallFixture::new();
    let first = fixture.ready("one.jpg", "2024-01-01T00:00:00Z");
    let second = fixture.ready("two.jpg", "2024-01-02T00:00:00Z");
    let third = fixture.ready("three.jpg", "2024-01-03T00:00:00Z");

    let ascending = fixture.catalog.wall_page(fixture.group, WallOrder::CapturedAscending, None, 2).unwrap();
    assert_eq!(ascending.items.iter().map(|item| item.id).collect::<Vec<_>>(), [first, second]);
    let tail = fixture.catalog.wall_page(fixture.group, WallOrder::CapturedAscending, ascending.next, 2).unwrap();
    assert_eq!(tail.items.iter().map(|item| item.id).collect::<Vec<_>>(), [third]);

    let descending = fixture.catalog.wall_page(fixture.group, WallOrder::CapturedDescending, None, 3).unwrap();
    assert_eq!(descending.items.iter().map(|item| item.id).collect::<Vec<_>>(), [third, second, first]);
}

#[test]
fn wall_page_excludes_pending_shapes_but_keeps_fallback_shapes() {
    let mut fixture = WallFixture::new();
    let pending = fixture.pending("pending.jpg");
    let ready = fixture.shaped("ready.jpg", ShapeStatus::Ready, 16, 9);
    let fallback = fixture.shaped("broken.jpg", ShapeStatus::Fallback, 4, 3);

    let page = fixture.catalog.wall_page(fixture.group, WallOrder::Provisional, None, 10).unwrap();
    assert!(!page.items.iter().any(|item| item.id == pending));
    assert_eq!(page.items.iter().map(|item| item.id).collect::<Vec<_>>(), [ready, fallback]);
    assert_eq!((page.items[1].width, page.items[1].height, page.items[1].shape_status), (4, 3, ShapeStatus::Fallback));
}
```

Define `WallFixture` and `ready_group` in the same test file. They create one folder group, attach every test asset to it, and apply `CatalogIndexRecord::Shaped` and `CatalogIndexRecord::Metadata` through the public batch API. They do not update SQLite through raw test-only SQL.

- [ ] **Step 2: Run the catalog test and verify the missing API failure**

Run: `cargo test -p photo-catalog --test wall_query`

Expected: FAIL because migration 4 and the wall projection types do not exist.

- [ ] **Step 3: Add migration 4**

```sql
-- crates/catalog/migrations/0004_wall_projection.sql
ALTER TABLE assets ADD COLUMN provisional_order INTEGER;
ALTER TABLE assets ADD COLUMN shape_status TEXT NOT NULL DEFAULT 'pending'
  CHECK(shape_status IN ('pending', 'ready', 'fallback'));

UPDATE assets SET provisional_order = rowid WHERE provisional_order IS NULL;

CREATE UNIQUE INDEX assets_library_provisional
  ON assets(library_id, provisional_order);
CREATE INDEX assets_library_capture
  ON assets(library_id, captured_at_utc, display_path, id);
CREATE INDEX derivatives_asset_kind_created
  ON derivatives(asset_id, kind, created_at DESC);

PRAGMA user_version = 4;
```

Append the migration to `MIGRATIONS`. In `upsert_asset_on`, assign a new order with `COALESCE((SELECT MAX(provisional_order) + 1 FROM assets WHERE library_id = ?2), 1)` only on insert. Preserve the stored order on conflict.

- [ ] **Step 4: Implement the wall repository and cursor keys**

```rust
// public shapes in crates/catalog/src/wall_repo.rs
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeStatus { Ready, Fallback }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WallOrder { Provisional, CapturedAscending, CapturedDescending }

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WallCursorKey {
    Provisional { order: u64, id: AssetId },
    Captured { captured_at_utc: String, display_path: String, id: AssetId },
}

pub struct WallCatalogPage {
    pub items: Vec<WallCatalogRecord>,
    pub next: Option<WallCursorKey>,
}
```

Use keyset SQL for all three orders and filter by `folder_group_id`, not only library ID. Query only `shape_status IN ('ready', 'fallback')`. Normalize every captured timestamp to UTC before storage in `catalog_writer.rs`, so textual order is chronological. Use `display_path` and `id` as settled tie-breakers.

Add `folder_group_id: Option<FolderGroupId>` to `NewAsset` and `AssetRecord`. `NewAsset::minimal` defaults it to `None`; active-source scans set it to the selected physical folder group. Preserve the group on enrichment and update it when a later scan explicitly classifies the asset into a group.

- [ ] **Step 5: Add derivative lookups without source paths**

Extend `DerivativeRecord` with `asset_id`, `kind`, and `cache_key`. Add:

```rust
pub fn derivatives_for_assets(
    &self,
    assets: &[AssetId],
    kind: &str,
) -> Result<Vec<DerivativeRecord>, CatalogError>;

pub fn find_derivative(
    &self,
    asset: AssetId,
    kind: &str,
    key: &str,
) -> Result<Option<DerivativeRecord>, CatalogError>;
```

Both methods return relative cache paths only.

- [ ] **Step 6: Run focused and regression tests**

Run: `cargo test -p photo-catalog --test wall_query`

Expected: PASS with 3 tests.

Run: `cargo test -p photo-catalog`

Expected: PASS, including migration, pagination, settings, and offline-retention tests.

- [ ] **Step 7: Commit the catalog projection**

```bash
git add crates/catalog
git commit -m "feat: add stable wall catalog projection"
```

---

### Task 2: Emit geometry before metadata and colour work

**Files:**
- Create: `crates/indexer/src/default_reader.rs`
- Modify: `crates/indexer/src/scanner.rs`
- Modify: `crates/indexer/src/events.rs`
- Modify: `crates/indexer/src/catalog_writer.rs`
- Modify: `crates/indexer/src/lib.rs`
- Modify: `crates/indexer/Cargo.toml`
- Modify: `crates/catalog/src/index_repo.rs`
- Modify: `crates/metadata/src/model.rs`
- Modify: `crates/metadata/src/probe.rs`
- Test: `crates/indexer/tests/progressive_scan.rs`

**Interfaces:**
- Consumes: migration-4 shape state, `EmbeddedExifReader`, `XmpSidecarReader`, and generation transactions.
- Produces: `DefaultMetadataReader`, `IndexEvent::ShapeReady`, `IndexEvent::ShapeFallback`, `IndexEvent::ColourReady`, staged `ScanProgress`, and generation-aware `CatalogWriter`.

- [ ] **Step 1: Replace the existing blocked-metadata test with a geometry-first test**

```rust
#[tokio::test]
async fn emits_shape_before_blocked_metadata_and_before_scan_completion() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("a.png"), [255, 0, 0]);
    write_png(&fixture.path().join("b.png"), [0, 0, 255]);
    let (reader, release) = BlockingMetadataReader::new();
    let indexer = Indexer::new(reader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();

    let shaped = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(IndexEvent::ShapeReady { width, height, .. }) = scan.events.recv().await {
                break (width, height);
            }
        }
    }).await.unwrap();

    assert_eq!(shaped, (1, 1));
    release.release();
    assert_eq!(scan.join().await.unwrap().discovered, 2);
}
```

Add these two concrete tests in the same file:

```rust
#[tokio::test]
async fn corrupt_shape_uses_four_by_three_and_still_reads_metadata() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("broken.jpg"), b"not a jpeg").unwrap();
    let reader = CountingMetadataReader::default();
    let reads = reader.reads.clone();
    let mut scan = Indexer::new(reader, empty_policy_engine()).start(ScanRequest::new(fixture.path())).unwrap();
    let mut fallback = None;
    while let Some(event) = scan.events.recv().await {
        match event {
            IndexEvent::ShapeFallback { width, height, .. } => fallback = Some((width, height)),
            IndexEvent::Completed(_) => break,
            _ => {}
        }
    }
    scan.join().await.unwrap();
    assert_eq!(fallback, Some((4, 3)));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn catalog_writer_commits_shape_before_later_metadata() {
    let mut fixture = CatalogWriterFixture::new();
    fixture.writer().apply_batch(&fixture.discovery_and_shape()).unwrap();
    let shaped = fixture.catalog().find_asset(fixture.asset_id()).unwrap().unwrap();
    assert_eq!((shaped.width, shaped.height), (Some(100), Some(50)));
    assert_eq!(shaped.rating, None);

    fixture.writer().apply_batch(&fixture.metadata_with_rating(4)).unwrap();
    assert_eq!(fixture.catalog().find_asset(fixture.asset_id()).unwrap().unwrap().rating, Some(4));
}
```

`CountingMetadataReader` owns `Arc<AtomicUsize>`. `CatalogWriterFixture` owns one in-memory catalog, library, generation, folder group, and asset so both calls use the same stable ID.

- [ ] **Step 2: Run the progressive scan test and verify it fails**

Run: `cargo test -p photo-indexer --test progressive_scan`

Expected: FAIL because `ShapeReady`, `ShapeFallback`, and independent pipeline stages do not exist.

- [ ] **Step 3: Separate scanner stages**

Change the event model to:

```rust
pub enum IndexEvent {
    Discovered { asset: NewAsset },
    ShapeReady { asset_id: AssetId, width: u32, height: u32, orientation: u16 },
    ShapeFallback { asset_id: AssetId, width: u32, height: u32, code: &'static str, message: String },
    ColourReady { asset_id: AssetId, representative_rgb: RepresentativeRgb },
    MetadataReady { asset_id: AssetId, metadata: ResolvedMetadata },
    Progress(ScanProgress),
    Warning { asset_id: Option<AssetId>, code: &'static str, message: String },
    Completed(ScanSummary),
}
```

Discovery feeds a bounded shape queue. Shape workers use `imagesize` and the embedded orientation tag, then emit shape immediately. A separate enrichment queue reads metadata and representative colour. Shape failure emits the fixed 4:3 fallback and does not skip metadata. Bound both queues at 64 items, use four shape workers and two enrichment workers while idle, and reduce enrichment to one worker during active interaction.

Extend `ScanRequest` with `library_root`, `selection_root`, and `folder_group_id`. Discovery walks `selection_root` but derives `RelativePathKey` and stable asset ID from `library_root`. Every discovered asset receives the selected group before its catalog event. This keeps identity correct when a user opens a subfolder inside an existing root.

- [ ] **Step 4: Add the default metadata reader**

`DefaultMetadataReader::read` merges embedded EXIF, adjacent XMP, filesystem creation time when available, and filesystem modified time. Add `MetadataBundle::extend` so merge behavior is explicit. Filesystem values use `MetadataSource::FilesystemBirth` and `MetadataSource::FilesystemModified`. A malformed sidecar becomes a warning while embedded and filesystem candidates remain usable.

- [ ] **Step 5: Make catalog batches generation-aware**

Construct `CatalogWriter::new(&mut catalog, library_id, generation)`. Within one transaction, discovery records update `last_seen_generation`; shape, colour, metadata, and warnings update the same stable asset IDs. Store capture dates as `value.with_timezone(&chrono::Utc).to_rfc3339()`.

- [ ] **Step 6: Run focused indexer verification**

Run: `cargo test -p photo-indexer --test progressive_scan`

Expected: PASS with geometry observed while metadata remains blocked.

Run: `cargo test -p photo-indexer`

Expected: PASS for progressive scan, scheduler priority, reconciliation, and watcher hints.

- [ ] **Step 7: Commit the progressive scanner**

```bash
git add crates/indexer crates/catalog/src/index_repo.rs crates/metadata
git commit -m "feat: publish photo geometry before enrichment"
```

---

### Task 3: Generate safe wall and screen derivatives

**Files:**
- Create: `crates/cache/src/image_derivative.rs`
- Create: `crates/cache/src/budget.rs`
- Create: `crates/cache/tests/image_derivative.rs`
- Create: `crates/cache/tests/cache_budget.rs`
- Modify: `crates/cache/src/lib.rs`
- Modify: `crates/cache/Cargo.toml`
- Modify: `crates/catalog/src/cache_repo.rs`

**Interfaces:**
- Consumes: `DerivativeSpec`, `DerivativeKey`, `CacheWriter`, `NewDerivative`, and a source path resolved outside the WebView.
- Produces: `ImageDerivativeGenerator::generate`, `GeneratedDerivative`, `ImageDerivativeError`, `CacheBudget::automatic`, `CacheBudget::prepare_write`, orientation normalization, and idempotent derivative registration.

- [ ] **Step 1: Write failing derivative tests**

```rust
#[test]
fn wall_and_screen_outputs_respect_long_edge_and_orientation() {
    let fixture = fixture_with_oriented_jpeg(1200, 800, 6);
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();

    let wall = generator.generate(&fixture.path, &spec(DerivativeKind::WallThumbnail, 1024, 6)).unwrap();
    let screen = generator.generate(&fixture.path, &spec(DerivativeKind::ScreenPreview, 4096, 6)).unwrap();

    assert_eq!(decode_size(cache.path().join(&wall.relative_path)), (683, 1024));
    assert_eq!(decode_size(cache.path().join(&screen.relative_path)), (800, 1200));
    assert!(wall.durable);
    assert!(!screen.durable);
}

#[test]
fn a_second_identical_request_reuses_the_atomic_cache_file() {
    let fixture = fixture_with_oriented_jpeg(1200, 800, 1);
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let requested = spec(DerivativeKind::WallThumbnail, 1024, 1);
    let first = generator.generate(&fixture.path, &requested).unwrap();
    let second = generator.generate(&fixture.path, &requested).unwrap();
    assert_eq!((second.key, second.relative_path), (first.key, first.relative_path));
    assert!(second.reused);
}

#[test]
fn generation_never_changes_the_source_file() {
    let fixture = fixture_with_oriented_jpeg(1200, 800, 1);
    let before_bytes = std::fs::read(&fixture.path).unwrap();
    let before_modified = std::fs::metadata(&fixture.path).unwrap().modified().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    generator.generate(&fixture.path, &spec(DerivativeKind::WallThumbnail, 1024, 1)).unwrap();
    generator.generate(&fixture.path, &spec(DerivativeKind::ScreenPreview, 4096, 1)).unwrap();
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before_bytes);
    assert_eq!(std::fs::metadata(&fixture.path).unwrap().modified().unwrap(), before_modified);
}
```

Define `fixture_with_oriented_jpeg`, `spec`, and `decode_size` in the test file. The fixture helper writes a real JPEG with a known pixel grid and returns its source signature. `spec` always includes the fixture asset ID, source signature, decoder version `image-0.25-v1`, and colour space `srgb`.

- [ ] **Step 2: Run the cache test and verify the missing generator failure**

Run: `cargo test -p photo-cache --test image_derivative`

Expected: FAIL because `ImageDerivativeGenerator` does not exist.

- [ ] **Step 3: Implement decoding, orientation, resize, and representative colour**

```rust
pub struct GeneratedDerivative {
    pub key: DerivativeKey,
    pub relative_path: PathBuf,
    pub size_bytes: u64,
    pub durable: bool,
    pub reused: bool,
    pub representative_rgb: RepresentativeRgb,
    pub content_type: &'static str,
}
```

Decode with the existing `image` crate, apply all eight EXIF orientation transforms, calculate representative RGB from a 32-pixel thumbnail, resize without upscaling, and encode JPEG. Use quality 82 for wall thumbnails and 90 for screen previews. Set the key colour-space component to `srgb` and decoder version to a project constant such as `image-0.25-v1`.

Write through `CacheWriter::write_atomic(key.sharded_path("jpg"), ...)`. The generator reads the source once per requested derivative and exposes no delete operation for source paths.

Add `CacheWriter::read_checked(&Path) -> Result<Vec<u8>, CacheError>` for the later protocol. It calls the existing containment and symlink checks before reading and accepts relative cache paths only.

- [ ] **Step 4: Register derivative rows idempotently**

Add `Catalog::upsert_derivative(&NewDerivative)`. On an existing immutable `cache_key`, verify asset, group, kind, path, size, and durability match; otherwise return `CatalogError::InvalidData`. Never overwrite a key with different identity.

- [ ] **Step 5: Enforce the automatic large-derivative budget**

Add `fs2 = "0.4.3"` to workspace dependencies. `CacheBudget::automatic(cache_root)` uses `fs2::total_space` and returns `min(total_space / 10, 100 * 1024 * 1024 * 1024)`. Before a non-durable write, compare cataloged non-durable bytes plus the estimated output against the limit. Use `EvictionPlanner` to remove the oldest eligible whole groups. Protect the active folder group and every group with an active write.

```rust
#[test]
fn automatic_limit_is_ten_percent_capped_at_one_hundred_gibibytes() {
    assert_eq!(CacheBudget::from_total_space(500 * GIB).limit_bytes(), 50 * GIB);
    assert_eq!(CacheBudget::from_total_space(2_000 * GIB).limit_bytes(), 100 * GIB);
}

#[test]
fn preparing_a_preview_write_evicts_one_old_group_not_individual_assets() {
    let mut fixture = BudgetFixture::with_limit(100);
    let old = fixture.group_with_preview_bytes(80, 1);
    let active = fixture.protected_group_with_preview_bytes(60, 2);
    let result = fixture.prepare_write(active, 40).unwrap();
    assert_eq!(result.evicted_groups, vec![old]);
    assert_eq!(fixture.catalog.derivative_count(old, false).unwrap(), 0);
    assert_eq!(fixture.catalog.derivative_count(active, false).unwrap(), 1);
}
```

`BudgetFixture` creates real cache files and matching derivative rows. The test must assert that durable wall thumbnails in the evicted group remain.

- [ ] **Step 6: Run cache verification**

Run: `cargo test -p photo-cache --test image_derivative`

Expected: PASS with both long-edge classes, reuse, orientation, and source immutability.

Run: `cargo test -p photo-cache --test cache_budget`

Expected: PASS for automatic sizing, group eviction, durable-thumbnail retention, and active-group protection.

Run: `cargo test -p photo-cache`

Expected: PASS for atomic writes, reconciliation, path containment, and whole-group eviction.

- [ ] **Step 7: Commit the derivative generator**

```bash
git add crates/cache crates/catalog/src/cache_repo.rs Cargo.lock
git commit -m "feat: generate cached photo derivatives"
```

---

### Task 4: Expose a host-neutral wall service and ordered updates

**Files:**
- Create: `crates/app-service/src/wall.rs`
- Create: `crates/app-service/src/scan.rs`
- Create: `crates/app-service/src/derivatives.rs`
- Create: `crates/app-service/tests/progressive_wall.rs`
- Modify: `crates/app-service/src/dto.rs`
- Modify: `crates/app-service/src/service.rs`
- Modify: `crates/app-service/src/lib.rs`
- Modify: `crates/app-service/Cargo.toml`
- Modify: `crates/core/src/library_service.rs`

**Interfaces:**
- Consumes: Tasks 1 through 3, active source selection, `IndexScheduler`, and cache policy.
- Produces: cloneable `AppService`, `WallQueryRequest`, `WallPage`, `WallAsset`, `DerivativeReference`, `DerivativeRequest`, `InteractionState`, `WallUpdate`, `AppService::query_wall`, `request_derivatives`, `subscribe_wall_updates`, and `set_interaction`.

- [ ] **Step 1: Write a failing end-to-end service test**

```rust
#[tokio::test]
async fn uncached_folder_emits_geometry_before_metadata_settles_and_reopens_from_cache() {
    let fixture = ProgressiveFixture::new_with_blocked_metadata(12);
    let service = AppService::open(fixture.config()).unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.open_recent(fixture.source()).await.unwrap();

    let batch = recv_until(&mut updates, |event| matches!(event, WallUpdate::CatalogBatch { assets, .. } if !assets.is_empty())).await;
    assert!(matches!(batch, WallUpdate::CatalogBatch { order_state: OrderState::Provisional, .. }));
    assert!(service.query_wall(WallQueryRequest::oldest_first()).await.unwrap().items.len() >= 4);

    fixture.release_metadata();
    recv_until(&mut updates, |event| matches!(event, WallUpdate::MetadataSettled { .. })).await;
    assert_eq!(service.query_wall(WallQueryRequest::oldest_first()).await.unwrap().order_state, OrderState::Settled);

    drop(service);
    let reopened = AppService::open(fixture.config()).unwrap();
    assert!(!reopened.query_wall(WallQueryRequest::oldest_first()).await.unwrap().items.is_empty());
}
```

Use these exact test names and assertions:

```rust
#[tokio::test]
async fn changing_direction_queries_sqlite_without_starting_another_scan() {
    let fixture = ProgressiveFixture::settled_with_dates(12);
    let service = AppService::open(fixture.config()).unwrap();
    let scans_before = fixture.scan_counter();
    let newest = service.query_wall(WallQueryRequest::newest_first()).await.unwrap();
    assert!(newest.items.windows(2).all(|pair| pair[0].captured_at_utc >= pair[1].captured_at_utc));
    assert_eq!(fixture.scan_counter(), scans_before);
}

#[tokio::test]
async fn one_visible_request_produces_one_ready_batch() {
    let fixture = ProgressiveFixture::settled_with_dates(8);
    let service = AppService::open(fixture.config()).unwrap();
    let ids = service.query_wall(WallQueryRequest::oldest_first()).await.unwrap().items.into_iter().map(|item| item.id).collect();
    let mut updates = service.subscribe_wall_updates();
    service.request_derivatives(DerivativeRequest::visible(ids)).await.unwrap();
    let ready = recv_until(&mut updates, |event| matches!(event, WallUpdate::DerivativesReady { .. })).await;
    assert!(matches!(ready, WallUpdate::DerivativesReady { derivatives } if derivatives.len() == 8));
}

#[tokio::test]
async fn offline_reopen_keeps_cached_references() {
    let fixture = ProgressiveFixture::settled_with_dates(4);
    let service = AppService::open(fixture.config()).unwrap();
    fixture.cache_all_wall_thumbnails(&service).await;
    drop(service);
    fixture.take_source_offline();
    let reopened = AppService::open(fixture.config()).unwrap();
    assert!(reopened.query_wall(WallQueryRequest::oldest_first()).await.unwrap().items.iter().all(|item| item.wall_thumbnail.is_some()));
}

#[test]
fn wall_dtos_never_serialize_native_paths() {
    let json = serde_json::to_string(&ProgressiveFixture::sample_wall_page()).unwrap();
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("\\\\server\\share"));
    assert!(!json.contains("relativeCachePath"));
}
```

`ProgressiveFixture` owns temporary data, cache, and source roots; creates real JPEGs; can block metadata through an injected test reader; and exposes counters through the app-service test dependency injection point. `recv_until` has a two-second timeout and fails with the last received event instead of hanging.

- [ ] **Step 2: Run the service integration test and verify it fails**

Run: `cargo test -p photo-app-service --test progressive_wall`

Expected: FAIL because wall operations and update subscription do not exist.

- [ ] **Step 3: Define the shared Rust DTOs**

```rust
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SortDirection { OldestFirst, NewestFirst }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallQueryRequest { pub cursor: Option<String>, pub limit: u32, pub direction: SortDirection }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivativeReference { pub asset_id: String, pub kind: DerivativeClass, pub key: String }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WallUpdate {
    CatalogBatch { assets: Vec<WallAsset>, order_state: OrderState, progress: ScanProgressDto },
    DerivativesReady { derivatives: Vec<DerivativeReference> },
    MetadataSettled { source_id: String },
    Progress { progress: ScanProgressDto },
    SourceUnavailable { source_id: String },
}
```

Limit pages to `1..=250`. Encode cursor keys as URL-safe base64 JSON and reject malformed, mismatched-direction, or oversized tokens with a bounded service error.

- [ ] **Step 4: Make `AppService` cloneable and background-safe**

Move the `LibraryService<RealSourceFs>` into `Arc<std::sync::Mutex<ServiceState>>`. Store the cache root, an `IndexScheduler`, a `tokio::sync::broadcast::Sender<WallUpdate>`, and one active scan cancellation handle. Keep catalog locks short. Source reads and derivative generation run in blocking workers without holding the catalog mutex.

`open_recent` persists the selection, ensures one physical folder group, returns bootstrap state, and starts the scan. Startup may query cached wall rows before a reconciliation scan begins.

- [ ] **Step 5: Orchestrate scan commits and update batches**

Begin a catalog generation before scanning. Collect events for at most 50 ms or 200 records, commit them transactionally, then publish one `CatalogBatch` for newly geometry-ready assets. Complete the generation only after discovery and enrichment finish. Publish exactly one `MetadataSettled` event for the first complete generation.

If the root cannot open, mark it offline, keep catalog rows, publish `SourceUnavailable`, and do not complete a missing-file generation.

- [ ] **Step 6: Orchestrate derivative priority**

`request_derivatives` accepts at most 250 stable IDs and a `Visible` or `NearViewport` priority. Resolve asset and source paths inside Rust, compute expected immutable specs, reuse ready rows, and queue missing work. Publish ready references in batches. After foreground indexing drains, enqueue screen previews for active and recently visible rows, then the remaining group while idle.

Before each screen-preview write, call `CacheBudget::prepare_write`. Keep the active group protected for the wall session. Wall thumbnails bypass the large-derivative budget because their durable byte count is reported separately.

- [ ] **Step 7: Run app-service verification**

Run: `cargo test -p photo-app-service --test progressive_wall`

Expected: PASS for provisional emission, settled ordering, derivative batching, offline restart, and path-free serialization.

Run: `cargo test -p photo-app-service`

Expected: PASS with checkpoint-1 bootstrap and settings behavior unchanged.

- [ ] **Step 8: Commit the shared wall service**

```bash
git add crates/app-service crates/core Cargo.lock
git commit -m "feat: orchestrate progressive wall service"
```

---

### Task 5: Add Tauri wall commands, channel, and cache-only protocol

**Files:**
- Create: `apps/desktop/src-tauri/src/protocol.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `apps/desktop/src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: Task 4 `AppService` operations and `WallUpdate` broadcast receivers.
- Produces: `query_wall`, `request_derivatives`, `set_wall_interaction`, `watch_wall_updates`, and `photo-derivative://localhost/<asset>/<kind>/<key>`.

- [ ] **Step 1: Write failing desktop command and protocol tests**

```rust
#[test]
fn derivative_protocol_rejects_malformed_or_mismatched_identity_without_paths() {
    let fixture = ProtocolFixture::with_ready_wall_thumbnail();
    let cases = vec![
        ("photo-derivative://localhost/not-a-uuid/wallThumbnail/key".to_owned(), 400),
        (fixture.uri_with_kind("unknown"), 400),
        (fixture.uri_with_key(&"x".repeat(257)), 400),
        (fixture.uri_for_missing_asset(), 404),
        (fixture.uri_with_key("wrong-key"), 404),
    ];
    for (uri, expected) in cases {
        let response = handle_derivative_request(&fixture.service, request(&uri));
        assert_eq!(response.status().as_u16(), expected);
        assert!(!String::from_utf8_lossy(response.body()).contains(fixture.source_root.to_string_lossy().as_ref()));
    }
}

#[tokio::test]
async fn wall_update_forwarder_sends_one_serializable_catalog_batch() {
    let (sender, receiver) = tokio::sync::broadcast::channel(4);
    let received = Arc::new(Mutex::new(Vec::<WallUpdate>::new()));
    let sink = received.clone();
    let channel = tauri::ipc::Channel::new(move |body| {
        sink.lock().unwrap().push(body.deserialize::<WallUpdate>().unwrap());
        Ok(())
    });
    let task = tokio::spawn(forward_wall_updates(receiver, channel));
    sender.send(sample_catalog_batch()).unwrap();
    drop(sender);
    task.await.unwrap();
    assert!(matches!(received.lock().unwrap().as_slice(), [WallUpdate::CatalogBatch { .. }]));
}
```

`ProtocolFixture` owns a temporary source, cache, catalog row, and matching derivative row. `request` builds `tauri::http::Request<Vec<u8>>`. Keep `handle_derivative_request` and `forward_wall_updates` free functions so these tests exercise the production parsing and forwarding paths without launching a window.

- [ ] **Step 2: Run desktop tests and verify missing handlers**

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`

Expected: FAIL because the wall commands and protocol module do not exist.

- [ ] **Step 3: Add typed asynchronous commands**

```rust
#[tauri::command]
async fn query_wall(request: WallQueryRequest, state: State<'_, DesktopState>) -> Result<WallPage, CommandError>;

#[tauri::command]
async fn request_derivatives(request: DerivativeRequest, state: State<'_, DesktopState>) -> Result<(), CommandError>;

#[tauri::command]
async fn set_wall_interaction(active: bool, state: State<'_, DesktopState>) -> Result<(), CommandError>;

#[tauri::command]
fn watch_wall_updates(on_event: tauri::ipc::Channel<WallUpdate>, state: State<'_, DesktopState>);
```

The watch command spawns one forwarder from `broadcast::Receiver` to `Channel::send` and exits when the JavaScript channel closes or lags. A lag produces one fresh progress snapshot rather than replaying an unbounded backlog.

- [ ] **Step 4: Register the custom protocol**

Register `photo-derivative` before `setup`. Parse exactly three path segments. Resolve the row through `AppService::read_derivative`, which revalidates the immutable key and cache-root containment. Return `image/jpeg`, `Cache-Control: public, max-age=31536000, immutable`, and `X-Content-Type-Options: nosniff`. Return empty bounded bodies for errors.

Add `photo-derivative:` and `http://photo-derivative.localhost` to `img-src` so the same adapter shape remains viable on Tauri's Windows origin form. Do not add `file:`, broad filesystem scopes, or an asset protocol serving source media.

- [ ] **Step 5: Run desktop verification**

Run: `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all --check`

Run: `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`

Expected: PASS with checkpoint-1 commands and new wall transport tests.

- [ ] **Step 6: Commit the desktop transport**

```bash
git add apps/desktop/src-tauri
git commit -m "feat: bridge progressive wall into tauri"
```

---

### Task 6: Extend the shared `PhotoService` contract and adapters

**Files:**
- Modify: `apps/interface/src/services/photoService.ts`
- Modify: `apps/interface/src/services/photoService.test.ts`
- Modify: `apps/interface/src/services/inMemoryPhotoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.ts`
- Modify: `apps/interface/src/services/tauriPhotoService.test.ts`

**Interfaces:**
- Consumes: Task 4 JSON DTO names and Task 5 commands.
- Produces: TypeScript `WallAsset`, `WallPage`, `WallUpdate`, `DerivativeReference`, `queryWall`, `requestDerivatives`, `setWallInteraction`, `watchWallUpdates`, and `derivativeUrl`.

- [ ] **Step 1: Write failing adapter contract tests**

```ts
it("maps wall operations and ordered channel updates", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const received: WallUpdate[] = [];
  const channels: FakeChannel<WallUpdate>[] = [];
  const service = createTauriPhotoService(
    recordingInvoke(calls),
    (listener) => {
      const channel = new FakeChannel(listener);
      channels.push(channel);
      return channel;
    },
  );
  const stop = service.watchWallUpdates((event) => received.push(event));

  await service.queryWall({ cursor: null, limit: 100, direction: "oldestFirst" });
  await service.requestDerivatives({ assetIds: ["asset-a"], priority: "visible" });
  await service.setWallInteraction(true);

  expect(calls.map(([name]) => name)).toEqual([
    "watch_wall_updates", "query_wall", "request_derivatives", "set_wall_interaction",
  ]);
  expect(service.derivativeUrl({ assetId: "asset-a", kind: "wallThumbnail", key: "abc" }))
    .toBe("photo-derivative://localhost/asset-a/wallThumbnail/abc");
  channels[0].emit(sampleCatalogBatch);
  expect(received).toEqual([sampleCatalogBatch]);
  stop();
  channels[0].emit(sampleProgressUpdate);
  expect(received).toEqual([sampleCatalogBatch]);
});
```

Define `recordingInvoke`, `FakeChannel`, `sampleCatalogBatch`, and `sampleProgressUpdate` in the test file. `FakeChannel` implements the same `onmessage` and IPC-serialization shape used by the adapter's injected `ChannelFactory`.

Add this in-memory ordering test:

```ts
it("emits geometry before metadata settles and stops after unsubscribe", async () => {
  const service = createInMemoryPhotoService({
    wallAssets: fixtureAssets,
    geometryDelayMs: 0,
    thumbnailDelayMs: 50,
    metadataDelayMs: 100,
  });
  const events: WallUpdate[] = [];
  const stop = service.watchWallUpdates((event) => events.push(event));
  await service.startFixtureScan();
  expect(events[0].kind).toBe("catalogBatch");
  expect(events.some((event) => event.kind === "metadataSettled")).toBe(false);
  await service.finishFixtureScan();
  expect(events.at(-1)?.kind).toBe("metadataSettled");
  stop();
  service.emitForTest(sampleProgressUpdate);
  expect(events.at(-1)?.kind).toBe("metadataSettled");
});
```

- [ ] **Step 2: Run unit tests and verify the missing contract failure**

Run: `npm test -- --run src/services/photoService.test.ts src/services/tauriPhotoService.test.ts`

Expected: FAIL because the wall methods and types are absent.

- [ ] **Step 3: Define the host-neutral TypeScript contract**

```ts
export interface PhotoService {
  readonly capabilities: PhotoServiceCapabilities;
  getBootstrapState(): Promise<BootstrapState>;
  chooseFolder(): Promise<ChooseFolderResult>;
  updateAppearance(appearance: Appearance): Promise<BootstrapState>;
  queryWall(request: WallQueryRequest): Promise<WallPage>;
  requestDerivatives(request: DerivativeRequest): Promise<void>;
  setWallInteraction(active: boolean): Promise<void>;
  watchWallUpdates(listener: (update: WallUpdate) => void): () => void;
  derivativeUrl(reference: DerivativeReference): string;
}
```

Use tagged unions matching Rust camel-case serialization. Keep derivative references free of paths.

- [ ] **Step 4: Implement a deterministic slow in-memory source**

Add options for `wallAssets`, `geometryDelayMs`, `thumbnailDelayMs`, and `metadataDelayMs`. Emit complete geometry batches in provisional order, then derivative batches, then one settled event. Use injected real fixture URLs for `derivativeUrl`; do not generate CSS-art or SVG stand-ins.

```ts
interface InMemoryOptions {
  selectedFolderName?: string;
  cancelFolderPicker?: boolean;
  wallAssets?: readonly InMemoryWallFixture[];
  geometryDelayMs?: number;
  thumbnailDelayMs?: number;
  metadataDelayMs?: number;
}

interface InMemoryPhotoService extends PhotoService {
  startFixtureScan(): Promise<void>;
  finishFixtureScan(): Promise<void>;
  emitForTest(update: WallUpdate): void;
}

type WallListener = (update: WallUpdate) => void;
const listeners = new Set<WallListener>();
const publish = (update: WallUpdate) => {
  for (const listener of listeners) listener(structuredClone(update));
};
```

- [ ] **Step 5: Implement the Tauri adapter channel**

Import `Channel` and `invoke` only in `tauriPhotoService.ts`. Add an injectable `ChannelFactory` whose production default constructs `new Channel(listener)`. Create one channel per listener. The unsubscribe closure replaces its handler with a no-op. Preserve the existing bounded native error mapping.

- [ ] **Step 6: Run interface unit verification**

Run: `npm test -- --run src/services/photoService.test.ts src/services/tauriPhotoService.test.ts`

Expected: PASS for checkpoint-1 and wall contract behavior.

Run: `npm run typecheck`

Expected: PASS.

- [ ] **Step 7: Commit the shared service contract**

```bash
git add apps/interface/src/services
git commit -m "feat: add progressive wall service contract"
```

---

### Task 7: Build the pure justified-row engine and wall state reducer

**Files:**
- Create: `apps/interface/src/wall/layoutJustifiedRows.ts`
- Create: `apps/interface/src/wall/layoutJustifiedRows.test.ts`
- Create: `apps/interface/src/wall/wallReducer.ts`
- Create: `apps/interface/src/wall/wallReducer.test.ts`

**Interfaces:**
- Consumes: `WallAsset` and settled or provisional batch state.
- Produces: `layoutJustifiedRows`, `JustifiedRow`, `PositionedWallAsset`, `wallReducer`, `WallState`, and `WallAction`.

- [ ] **Step 1: Write failing geometry tests**

```ts
it("fills complete rows without changing source aspect ratios", () => {
  const rows = layoutJustifiedRows(assets([1.5, 1, 2, 0.75]), {
    containerWidth: 1000, targetRowHeight: 220, gap: 4, sourceComplete: false,
  });
  expect(rows).toHaveLength(1);
  expect(rows[0].width).toBeCloseTo(1000, 5);
  for (const item of rows[0].items) {
    expect(item.width / item.height).toBeCloseTo(item.asset.width / item.asset.height, 5);
  }
});

it("holds an incomplete row until the source completes", () => {
  expect(layoutJustifiedRows(assets([1]), options({ sourceComplete: false }))).toEqual([]);
  expect(layoutJustifiedRows(assets([1]), options({ sourceComplete: true }))[0].justified).toBe(false);
});

it("accounts for gaps and leaves tile geometry unchanged when a derivative arrives", () => {
  const source = assets([1.5, 0.75, 2]);
  const before = layoutJustifiedRows(source, options({ containerWidth: 900, gap: 4 }));
  const refined = source.map((asset, index) => index === 0 ? {
    ...asset,
    wallThumbnail: { assetId: asset.id, kind: "wallThumbnail" as const, key: "ready" },
  } : asset);
  const after = layoutJustifiedRows(refined, options({ containerWidth: 900, gap: 4 }));
  expect(after.map((row) => row.items.map(({ width, height }) => [width, height])))
    .toEqual(before.map((row) => row.items.map(({ width, height }) => [width, height])));
  expect(before[0].items.reduce((sum, item) => sum + item.width, 0) + 8).toBeCloseTo(900, 5);
});

it("rejects non-positive source dimensions", () => {
  expect(() => layoutJustifiedRows([{ ...assets([1])[0], width: 0 }], options()))
    .toThrow("Wall asset dimensions must be positive");
});
```

Define `assets(aspectRatios)` and `options(overrides)` at the top of the test file. `assets` returns stable IDs, positive integer dimensions, alternating landscape and portrait records, and no derivative by default. `options` supplies a 1000-pixel container, 220-pixel target row, 4-pixel gap, and `sourceComplete: false`.

```ts
it("merges idempotently, refines in place, and resets once at settlement", () => {
  const provisional = reduce(initialWallState, {
    type: "catalogBatch",
    assets: [wallAsset("a", 1), wallAsset("b", 2), wallAsset("a", 1)],
    orderState: "provisional",
  });
  expect(provisional.items.map((item) => item.id)).toEqual(["a", "b"]);
  const refined = reduce(provisional, {
    type: "derivativesReady",
    derivatives: [{ assetId: "a", kind: "wallThumbnail", key: "ready" }],
  });
  expect(refined.items[0]).toMatchObject({ id: "a", width: provisional.items[0].width });
  expect(refined.items[0].wallThumbnail?.key).toBe("ready");
  const settled = reduce(refined, {
    type: "metadataSettled",
    assets: [refined.items[1], refined.items[0]],
  });
  expect(settled.items.map((item) => item.id)).toEqual(["b", "a"]);
  expect(reduce(settled, { type: "metadataSettled", assets: [settled.items[1], settled.items[0]] })).toBe(settled);
  const reversed = reduce(settled, { type: "setDirection", direction: "newestFirst" });
  expect(reversed).toMatchObject({ items: [], cursor: null, scrollEpoch: settled.scrollEpoch + 1 });
});
```

Define `wallAsset` in `wallReducer.test.ts` with stable dimensions, provisional order, date, and no derivative.

- [ ] **Step 2: Run layout tests and verify missing modules**

Run: `npm test -- --run src/wall/layoutJustifiedRows.test.ts src/wall/wallReducer.test.ts`

Expected: FAIL because both modules are absent.

- [ ] **Step 3: Implement the row algorithm**

Accumulate aspect ratios until the candidate row height is at or below the target. Compute `availableWidth = containerWidth - gap * (count - 1)`, set `rowHeight = availableWidth / sum(aspectRatios)`, and assign the final item the rounding remainder so each complete row ends exactly at the container edge. Return CSS-pixel numbers; rendering owns no layout calculation.

- [ ] **Step 4: Implement stable wall state**

The reducer merges catalog and derivative batches by asset ID. Provisional catalog batches append according to `provisionalOrder`. A derivative batch replaces only the reference field. `metadataSettled` replaces the ordered list once. `setDirection` clears items, cursor, and scroll epoch so the UI returns to the top.

- [ ] **Step 5: Run wall-core verification**

Run: `npm test -- --run src/wall/layoutJustifiedRows.test.ts src/wall/wallReducer.test.ts`

Expected: PASS with exact complete-row width and unchanged geometry after derivative refinement.

- [ ] **Step 6: Commit the wall engine**

```bash
git add apps/interface/src/wall
git commit -m "feat: add stable justified wall engine"
```

---

### Task 8: Render the progressive wall with real photos

**Files:**
- Create: `apps/interface/src/app/usePhotoWall.ts`
- Create: `apps/interface/src/components/PhotoWallCanvas.tsx`
- Create: `apps/interface/src/components/WallToolbar.tsx`
- Create: `apps/interface/src/components/JustifiedWall.tsx`
- Create: `apps/interface/src/components/PhotoTile.tsx`
- Create: `apps/interface/src/components/PhotoWall.browser.test.tsx`
- Create: `apps/interface/src/styles/photoWall.module.css`
- Create: `apps/interface/public/demo-photos/coast.jpg`
- Create: `apps/interface/public/demo-photos/forest.jpg`
- Create: `apps/interface/public/demo-photos/city.jpg`
- Create: `apps/interface/public/demo-photos/mountain.jpg`
- Create: `apps/interface/public/demo-photos/portrait.jpg`
- Create: `apps/interface/public/demo-photos/interior.jpg`
- Modify: `apps/interface/src/components/SourceCanvas.tsx`
- Modify: `apps/interface/src/components/AppShell.tsx`
- Modify: `apps/interface/src/styles/appShell.module.css`
- Modify: `apps/interface/src/styles/tokens.css`

**Interfaces:**
- Consumes: Tasks 6 and 7 plus the existing Canvas First shell and theme tokens.
- Produces: the checkpoint-2 user experience, real-photo in-memory fixtures, visible derivative priority, vertical incremental loading, and oldest/newest control.

- [ ] **Step 1: Create six photographic fixtures with ImageGen**

Use the imagegen skill for six separate 3:2 or 2:3 JPEGs. Keep the art direction consistent: restrained documentary photography, natural light, no text, no logos, no collage, varied dominant colours, and a mix of landscape and portrait orientation. Save each result to its named file above. These are demo and browser-test media, not production placeholders.

- [ ] **Step 2: Write the failing slow-source browser tests**

```tsx
it("reveals complete rows before metadata settles and keeps tile geometry during refinement", async () => {
  const service = createSlowWallService(realFixtureAssets);
  const screen = await renderWall(service);
  const firstTile = screen.getByRole("img", { name: "Coast" });

  await expect.element(screen.getByText(/Indexing/)).toBeVisible();
  const before = firstTile.element().parentElement!.getBoundingClientRect();
  service.releaseVisibleThumbnails();
  await expect.element(firstTile).toBeVisible();
  const after = firstTile.element().parentElement!.getBoundingClientRect();
  expect(after).toEqual(before);
  expect(service.metadataSettled).toBe(false);
});

it("switches to newest first and resets the scroll container", async () => {
  const service = createSlowWallService(realFixtureAssets, { initiallySettled: true });
  const screen = await renderWall(service);
  const wall = screen.getByRole("region", { name: "Photos" }).element();
  wall.scrollTop = 500;
  wall.dispatchEvent(new Event("scroll"));
  await screen.getByRole("button", { name: "Newest first" }).click();
  await expect.poll(() => wall.scrollTop).toBe(0);
  await expect.element(screen.getAllByRole("img")[0]).toHaveAttribute("alt", "Interior");
});

it("loads another bounded page near the end and requests visible work first", async () => {
  const service = createSlowWallService(
    [...realFixtureAssets, ...realFixtureAssets, ...realFixtureAssets],
    { pageSize: 6 },
  );
  const screen = await renderWall(service);
  service.intersectLoadSentinel();
  await expect.poll(() => service.queryRequests.length).toBe(2);
  expect(service.derivativeRequests[0].priority).toBe("visible");
  expect(service.derivativeRequests.some((request) => request.priority === "nearViewport")).toBe(true);
  expect(screen.getByRole("region", { name: "Photos" }).element().scrollHeight)
    .toBeGreaterThan(screen.getByRole("region", { name: "Photos" }).element().clientHeight);
});

it("renders fallback and reduced-motion states accessibly", async () => {
  document.documentElement.style.setProperty("--fade-duration", "0ms");
  for (const [width, height] of [[1440, 1024], [834, 1194], [390, 844]] as const) {
    await page.viewport(width, height);
    const service = createSlowWallService([fallbackAsset, ...realFixtureAssets]);
    const screen = await renderWall(service);
    const fallback = screen.getByTestId("photo-fallback").element().getBoundingClientRect();
    await expect.element(screen.getByText("File unavailable")).toBeVisible();
    expect(fallback.width / fallback.height).toBeCloseTo(4 / 3, 2);
    expect(getComputedStyle(screen.getByTestId("photo-row-0").element()).animationDuration).toBe("0s");
    expect(screen.getByRole("status").element().textContent).toMatch(/Indexing|photos ready/);
    expect(seriousViolations(await axe.run(document))).toEqual([]);
    screen.unmount();
  }
  document.documentElement.style.removeProperty("--fade-duration");
});
```

Define `renderWall`, `createSlowWallService`, `realFixtureAssets`, `fallbackAsset`, and `seriousViolations` in the test file. `realFixtureAssets` uses the six generated JPEG URLs with capture dates and dimensions read from those files. The fake exposes controlled intersection triggers and request logs, so the tests do not depend on arbitrary sleeps. Add one empty-page test that asserts `No photos found`, and one colour-stage test that asserts `rgb(34, 86, 112)` before releasing the JPEG.

- [ ] **Step 3: Run the browser tests and verify the missing wall failure**

Run: `npm run test:browser -- --run src/components/PhotoWall.browser.test.tsx`

Expected: FAIL because wall components and the hook do not exist.

- [ ] **Step 4: Implement the wall controller hook**

Subscribe once per active source. Query the first 100 records, merge update batches, and request the next page when the load sentinel enters the scroll container. Report pointer, keyboard, touch, wheel, and resize activity through a debounced `setWallInteraction(true)` followed by `false` after 200 ms of quiet.

Request wall derivatives once per visible asset-ID batch. Use `IntersectionObserver` for visibility and scroll-ahead detection. Do not call the service once per tile.

- [ ] **Step 5: Implement the Canvas First wall**

Replace the selected-folder holding state with `PhotoWallCanvas`. Keep the existing rail and compact toolbar. Put the date direction control at the right of the source title before Appearance. Use clear labels, `Oldest first` and `Newest first`, with a line icon that supports rather than replaces the text.

Rows fade in with `var(--fade-duration)`. Tiles use fixed inline width and height from the layout engine. The neutral layer, representative-colour layer, and `<img>` stack in the same grid cell. The image starts at opacity zero and crossfades after `onLoad`. Set `decoding="async"`, `draggable={false}`, and an accessible filename or display name.

- [ ] **Step 6: Run interface verification**

Run: `npm run check`

Run: `npm run typecheck`

Run: `npm test`

Run: `npm run test:browser`

Run: `npm run --workspace @photo-viewer/interface build`

Expected: PASS with checkpoint-1 shell tests and new progressive wall tests.

- [ ] **Step 7: Commit the wall interface and fixtures**

```bash
git add apps/interface
git commit -m "feat: render progressive justified photo wall"
```

---

### Task 9: Verify the real macOS vertical slice and document the checkpoint

**Files:**
- Create: `docs/superpowers/verification/2026-08-25-macos-progressive-photo-wall.md`
- Create: `docs/superpowers/verification/assets/2026-08-25-wall-provisional.png`
- Create: `docs/superpowers/verification/assets/2026-08-25-wall-refined.png`
- Create: `docs/superpowers/verification/assets/2026-08-25-wall-sort.gif`
- Modify: `README.md`
- Modify: `.github/workflows/ci.yml` only if a new command from Tasks 1 through 8 is not already covered

**Interfaces:**
- Consumes: the complete checkpoint implementation and `apps/interface/public/demo-photos`.
- Produces: fresh whole-repository evidence, a runnable unsigned `.app`, and a user-reviewable demonstration record.

- [ ] **Step 1: Add a clean-profile demo command to the README**

Document:

```bash
PHOTO_VIEWER_PROFILE=wall-demo npm run desktop:dev
```

The demo opens `apps/interface/public/demo-photos` through the native picker. Document where named-profile catalog and cache state live, how to select a new profile name for another clean run, and that no command deletes source media.

- [ ] **Step 2: Run fresh root Rust verification**

Run: `cargo fmt --all --check`

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Run: `cargo test --workspace --all-features`

Run: `cargo test -p catalog-bench --test benchmark_smoke`

Expected: every command exits 0.

- [ ] **Step 3: Run fresh interface verification**

Run: `npm run check`

Run: `npm run typecheck`

Run: `npm test`

Run: `npm run test:browser`

Run: `npm run --workspace @photo-viewer/interface build`

Expected: every command exits 0.

- [ ] **Step 4: Run fresh desktop verification and build**

Run: `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all --check`

Run: `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`

Run: `npm run desktop:build -- --bundles app`

Expected: every command exits 0 and `apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app` exists.

- [ ] **Step 5: Demonstrate uncached, refined, sorted, and cached states**

Launch the clean profile, choose `apps/interface/public/demo-photos`, and capture the wall while some rows remain colour-backed. Capture the same wall after thumbnails refine. Record switching oldest/newest and vertical scrolling. Quit, make the fixture source temporarily unavailable without deleting it, relaunch the same profile, and verify the cached wall appears before the unavailable cue.

Inspect the profile cache and record that both `wall_thumbnail` and `screen_preview` derivative rows exist. Do not expose absolute profile or source paths in screenshots or the verification document.

- [ ] **Step 6: Write the verification record**

Record the exact commit, command outputs and test counts, first cached-paint timing, cached screen-preview timing, screenshots, motion capture, cache-tier evidence, source-read-only audit, and known checkpoint-3 limits. State any missed target plainly.

- [ ] **Step 7: Commit verification and documentation**

```bash
git add README.md .github/workflows/ci.yml docs/superpowers/verification
git commit -m "test: verify progressive macos photo wall"
```

## Final checkpoint review

After Task 9, invoke `superpowers:requesting-code-review`. The reviewer checks the implementation against the approved specification and this plan, with special attention to source immutability, provisional-order stability, geometry-before-metadata behavior, derivative path containment, transport isolation, and the real macOS demonstration. Fix findings through test-driven changes, rerun the full Task 9 verification set, and use `superpowers:verification-before-completion` before reporting success.
