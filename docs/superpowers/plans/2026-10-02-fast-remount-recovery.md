# Fast remount recovery implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore a remounted saved folder immediately and reconcile unchanged
photos using filesystem metadata without reopening or decoding their content.

**Architecture:** A successful saved-folder probe first clears only
`root_offline` catalog state. The scan request then carries a catalog snapshot
of previously settled assets. Discovery compares file signatures and sends only
new, changed, or explicitly refreshable assets through shape and metadata work.
The interface filters stale source warnings against the live folder context.

**Tech stack:** Rust 1.97.1, Tokio, rusqlite, React 19, TypeScript 7, Vitest
browser tests

**Spec:** `docs/superpowers/specs/2026-10-02-fast-remount-recovery-design.md`

## Global constraints

- Source photo folders remain read-only. Tests use temporary fixtures only.
- A successful access probe clears `root_offline`, never `missing` or
  `unreadable`.
- Unchanged assets receive discovery and generation membership but no shape,
  colour, metadata, or decoder work.
- New, changed, incomplete, and forced-refresh assets use the current full
  pipeline.
- Keep generation fencing, cancellation, inventory validation, and missing-file
  settlement unchanged.
- Do not add a schema migration or loosen HEIC feature defaults.

## Review focus

- A late successful folder probe must not overwrite a newer failed probe. Task
  3 adds a generation-order test.
- Recovering one child folder must not clear an unavailable sibling. Task 1
  adds a scoped catalog test.
- A matching signature must not suppress missing shape data or forced HEIF
  refresh. Tasks 1 and 2 cover both inputs.
- A file removed during reconciliation must still become missing only after a
  successful generation. Task 3 retains the generation completion path and
  adds an end-to-end removal test.
- Clearing a source warning must not hide derivative or unreadable warnings.
  Task 4 tests each warning class.

---

### Task 1: Catalog recovery state and known-asset snapshot

**Files:**

- Modify: `crates/catalog/src/generation_repo.rs`
- Modify: `crates/catalog/src/lib.rs`
- Modify: `crates/catalog/tests/recovery_state.rs`

**Interfaces:**

- Consumes: existing `library_roots.availability`,
  `folder_group_assets.last_seen_generation`, asset signature and shape fields,
  and completed group generations.
- Produces:
  `Catalog::mark_root_available(LibraryId) -> Result<u64, CatalogError>`,
  `Catalog::mark_group_available(LibraryId, FolderGroupId) -> Result<u64, CatalogError>`,
  and
  `Catalog::reconciliation_assets_for_group(LibraryId, FolderGroupId) -> Result<Vec<ReconciliationAssetRecord>, CatalogError>`.
- Produces: public `ReconciliationAssetRecord { id: AssetId, media_kind:
  MediaKind, signature: FileSignature, shape_status: ShapeStatus }`.

- [ ] **Step 1: Write failing recovery-state tests**

Add these tests to `crates/catalog/tests/recovery_state.rs`:

```rust
#[test]
fn availability_recovery_root_clears_only_root_offline_assets() {
    // root becomes available; root_offline -> available;
    // missing and unreadable remain unchanged.
}

#[test]
fn availability_recovery_child_is_scoped_to_its_membership() {
    // recovered child -> available; offline sibling stays root_offline.
}
```

- [ ] **Step 2: Run the recovery tests and verify RED**

Run: `cargo test -p photo-catalog --test recovery_state availability_recovery_ -- --nocapture`

Expected: FAIL because the two availability methods do not exist.

- [ ] **Step 3: Implement the two transactional availability methods**

`mark_root_available` updates the library root timestamp and only assets with
`availability = 'root_offline'`. `mark_group_available` proves the library root
available but limits the asset update through `folder_group_assets`. Both verify
library/group identity and return the number of recovered assets.

- [ ] **Step 4: Run the recovery tests and verify GREEN**

Run the Step 2 command.

Expected: both tests PASS.

- [ ] **Step 5: Write the failing reconciliation-snapshot test**

```rust
#[test]
fn reconciliation_snapshot_contains_only_assets_from_the_latest_completed_group_generation() {
    // Assert exact id, media kind, signature, and shape status.
    // Exclude an asset advanced by a newer incomplete generation.
}
```

- [ ] **Step 6: Run the snapshot test and verify RED**

Run: `cargo test -p photo-catalog --test recovery_state reconciliation_snapshot_contains_only_assets_from_the_latest_completed_group_generation -- --exact --nocapture`

Expected: FAIL because `reconciliation_assets_for_group` does not exist.

- [ ] **Step 7: Implement and export `ReconciliationAssetRecord`**

Query membership against the latest completed generation for that group. Return
an empty vector when no completed group generation exists. Decode signatures
with the catalog's existing checked conversions.

- [ ] **Step 8: Run the catalog test target and commit**

Run: `cargo test -p photo-catalog --test recovery_state`

Expected: PASS.

```bash
git add crates/catalog/src/generation_repo.rs crates/catalog/src/lib.rs crates/catalog/tests/recovery_state.rs
git commit -m "feat: add catalog remount recovery state"
```

### Task 2: Indexer unchanged-asset fast path

**Files:**

- Modify: `crates/indexer/src/scanner.rs`
- Modify: `crates/indexer/src/lib.rs`
- Modify: `crates/indexer/tests/progressive_scan.rs`

**Interfaces:**

- Consumes: discovered `AssetId` and `FileSignature` before shape probing.
- Produces: public `KnownAsset { id: AssetId, signature: FileSignature,
  enrichment_required: bool }` and
  `ScanRequest::with_known_assets(impl IntoIterator<Item = KnownAsset>) -> ScanRequest`.
- Produces: unchanged assets emit `IndexEvent::Discovered`, advance shaped and
  enriched progress, and bypass shape and metadata workers.

- [ ] **Step 1: Write failing fast-path tests**

Add focused tests to `crates/indexer/tests/progressive_scan.rs`:

```rust
#[tokio::test]
async fn unchanged_known_asset_skips_shape_and_metadata_reads() {
    // Same signature, enrichment_required false.
    // Assert Discovered and Completed, no shape/colour/metadata event,
    // metadata reader calls == 0, and completed progress is 1/1/1.
}

#[tokio::test]
async fn changed_or_forced_asset_uses_full_enrichment_pipeline() {
    // Changed signature and enrichment_required true each call the reader.
}
```

- [ ] **Step 2: Run both tests and verify RED**

Run: `cargo test -p photo-indexer --test progressive_scan known_asset -- --nocapture`

Expected: FAIL because `KnownAsset` and `with_known_assets` do not exist.

- [ ] **Step 3: Add the known-asset request contract**

Store known assets in a hash map keyed by `AssetId`. Keep an empty map as the
default so existing callers retain current behaviour.

- [ ] **Step 4: Add the signature decision before enrichment admission**

After emitting `Discovered`, compare the discovered signature with the known
record. On an eligible match, increment shaped and enriched progress for a
wall-viewable photo and continue without taking a scheduler permit. All other
assets enter the existing shape and metadata queues unchanged.

- [ ] **Step 5: Run the focused tests and verify GREEN**

Run the Step 2 command.

Expected: both tests PASS with zero metadata reads for the unchanged case.

- [ ] **Step 6: Add the ignored 5,000-file diagnostic**

```rust
#[tokio::test]
#[ignore = "local remount performance diagnostic"]
async fn unchanged_5000_asset_reconciliation_reports_zero_content_reads() {
    // Print elapsed time and assert reader calls == 0 and 5,000 completed.
}
```

The diagnostic reports elapsed time but does not assert a wall-clock limit.

- [ ] **Step 7: Run the indexer suite and commit**

Run: `cargo test -p photo-indexer`

Expected: PASS, with the performance diagnostic ignored.

```bash
git add crates/indexer/src/scanner.rs crates/indexer/src/lib.rs crates/indexer/tests/progressive_scan.rs
git commit -m "feat: skip unchanged assets during reconciliation"
```

### Task 3: Saved-folder recovery and scan wiring

**Files:**

- Modify: `crates/app-service/src/saved_folders.rs`
- Modify: `crates/app-service/src/gallery.rs`
- Modify: `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**

- Consumes: Task 1 catalog recovery/snapshot methods and Task 2 `KnownAsset`
  request builder.
- Produces: an accepted `FolderProbeOutcome::Available` updates catalog and
  saved-folder availability before scan completion.
- Produces: `start_runtime_scan` maps `ReconciliationAssetRecord` to
  `KnownAsset`; non-ready shapes and HEIF assets under a required HEIF refresh
  set `enrichment_required = true`.

- [ ] **Step 1: Write failing saved-folder recovery tests**

Add tests in the existing `saved_folders.rs` test module:

```rust
#[tokio::test]
async fn saved_folder_recovery_restores_catalog_before_scan_completion() {
    // Mark the source offline, prove it readable, and assert snapshot plus
    // catalog availability immediately without waiting for indexing.
}

#[tokio::test]
async fn saved_folder_recovery_ignores_stale_success_after_newer_failure() {
    // Complete generation N+1 as unavailable before generation N succeeds.
    // Assert catalog and snapshot stay unavailable.
}
```

- [ ] **Step 2: Run the saved-folder tests and verify RED**

Run: `cargo test -p photo-app-service saved_folder_recovery_ -- --nocapture`

Expected: the first test sees `root_offline`; the second exposes the catalog
mutation happening before generation acceptance.

- [ ] **Step 3: Apply accepted access results atomically under service state**

Move availability mutation inside the existing generation check in
`check_saved_folder`. For `Available`, call the root or group recovery method
before publishing the updated folder snapshot. For unavailable outcomes, keep
the current scoped offline mutation and wall event. Ignore stale replies for
both catalog and runtime state.

- [ ] **Step 4: Run the saved-folder tests and verify GREEN**

Run the Step 2 command.

Expected: both tests PASS.

- [ ] **Step 5: Write failing scan-integration tests**

Add to `crates/app-service/tests/progressive_wall.rs`:

```rust
#[tokio::test]
async fn remounted_unchanged_folder_finishes_without_metadata_reads() {
    // Complete an initial scan, mark root offline, restore it, and rescan.
    // Assert the spy reader count does not increase on the second scan.
}

#[tokio::test]
async fn remounted_scan_marks_a_removed_asset_missing_only_on_completion() {
    // Remove one disposable fixture between scans; it remains retained during
    // the generation and becomes missing after successful completion.
}
```

- [ ] **Step 6: Run the integration tests and verify RED**

Run: `cargo test -p photo-app-service --test progressive_wall remounted_ -- --nocapture`

Expected: unchanged assets still increment the reader count.

- [ ] **Step 7: Load and pass the known-asset snapshot**

In `start_runtime_scan`, read the snapshot before beginning the new generation.
Map shape status and the existing group HEIF refresh flag into
`enrichment_required`, then attach the records to `ScanRequest`. Do not change
completion, cancellation, or inventory validation.

- [ ] **Step 8: Run the application-service tests and commit**

Run: `cargo test -p photo-app-service saved_folders -- --nocapture && cargo test -p photo-app-service --test progressive_wall remounted_ -- --nocapture`

Expected: PASS.

```bash
git add crates/app-service/src/saved_folders.rs crates/app-service/src/gallery.rs crates/app-service/tests/progressive_wall.rs
git commit -m "fix: restore remounted folders before reconciliation"
```

### Task 4: Remove stale source-warning badges

**Files:**

- Modify: `apps/interface/src/folders/SourceAvailabilityContext.tsx`
- Modify: `apps/interface/src/components/PhotoTile.tsx`
- Modify: `apps/interface/src/components/PickRow.tsx`
- Modify: `apps/interface/src/components/PhotoTile.browser.test.tsx`
- Modify: `apps/interface/src/components/PicksPanel.browser.test.tsx`

**Interfaces:**

- Consumes: `SourceUnavailableContext` where `false` means the current folder
  probe explicitly succeeded.
- Produces: `visibleImageWarning(context: boolean | null, asset: WallAsset)`
  returns `null` only for a stale `sourceUnavailable` warning under an explicitly
  available context. Other warnings pass through unchanged.

- [ ] **Step 1: Write failing tile and pick-row browser tests**

```tsx
it("hides a stale source warning after the folder becomes available", () => {
  // Context false plus sourceUnavailable: no source badge and no generic badge.
});

it("keeps item warnings after the folder becomes available", () => {
  // Context false plus derivative warning: generic warning remains.
  // missing/unreadable availability remains unavailable.
});
```

Add the equivalent stale-source assertion for a pick row in
`PicksPanel.browser.test.tsx`.

- [ ] **Step 2: Run the browser tests and verify RED**

Run: `npm run test:browser --workspace @photo-viewer/interface -- src/components/PhotoTile.browser.test.tsx src/components/PicksPanel.browser.test.tsx`

Expected: the stale source warning appears as `Photo preview warning` and as a
pick-row source warning.

- [ ] **Step 3: Add and use `visibleImageWarning`**

Keep `imageSourceUnavailable` as the source-badge decision. Use the new helper
for the generic tile badge and pick-row warning text, with the context read once
per component. Do not suppress missing, unreadable, preview, or derivative
warnings.

- [ ] **Step 4: Run the focused browser tests and verify GREEN**

Run the Step 2 command.

Expected: PASS.

- [ ] **Step 5: Run interface checks and commit**

Run: `npm run typecheck && npm test && npm run test:browser --workspace @photo-viewer/interface -- src/components/PhotoTile.browser.test.tsx src/components/PicksPanel.browser.test.tsx`

Expected: PASS.

```bash
git add apps/interface/src/folders/SourceAvailabilityContext.tsx apps/interface/src/components/PhotoTile.tsx apps/interface/src/components/PickRow.tsx apps/interface/src/components/PhotoTile.browser.test.tsx apps/interface/src/components/PicksPanel.browser.test.tsx
git commit -m "fix: clear stale source warning badges"
```

### Task 5: Whole-change verification

**Files:**

- Modify only if a verification failure needs a test-first fix.

**Interfaces:**

- Consumes: Tasks 1 through 4.
- Produces: a clean Rust and interface verification result plus one local
  5,000-file diagnostic measurement.

- [ ] **Step 1: Run formatting and static checks**

Run: `cargo fmt --all -- --check && cargo clippy -p photo-catalog -p photo-indexer -p photo-app-service --all-targets -- -D warnings && npm run check && npm run typecheck`

Expected: PASS with no warnings.

- [ ] **Step 2: Run the affected full test suites**

Run: `cargo test -p photo-catalog -p photo-indexer -p photo-app-service && npm test && npm run test:browser`

Expected: PASS.

- [ ] **Step 3: Run the local 5,000-file diagnostic**

Run: `cargo test -p photo-indexer --test progressive_scan unchanged_5000_asset_reconciliation_reports_zero_content_reads -- --ignored --exact --nocapture`

Expected: PASS, 5,000 reconciled assets, zero content-reader calls, and an
elapsed-time report. Record the measured duration in the task ledger.

- [ ] **Step 4: Run the interface production build**

Run: `npm run --workspace @photo-viewer/interface build`

Expected: PASS.

- [ ] **Step 5: Commit any test-first verification fixes**

If no fix was required, make no empty commit. If a fix was required, stage only
its test and implementation and use:

```bash
git commit -m "fix: close remount recovery verification gap"
```
