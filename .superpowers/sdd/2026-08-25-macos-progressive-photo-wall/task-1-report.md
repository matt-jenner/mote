# Task 1 report

## Implementation summary

Added migration 4 for append-only per-library provisional ordering, shape status, and wall/derivative indexes. Added the wall repository with folder-group filtering, keyset cursors, provisional ordering, and captured-date ordering. Extended assets with folder-group identity and projection fields, shape updates, generation completion lookup, and derivative lookup APIs that return relative cache paths.

## Files changed

- `crates/catalog/migrations/0004_wall_projection.sql`
- `crates/catalog/src/{asset_repo,cache_repo,generation_repo,index_repo,lib,migrate,wall_repo}.rs`
- `crates/catalog/tests/wall_query.rs`

## Self-review

The wall query preserves provisional order on upsert conflicts, scopes all pages to `folder_group_id`, excludes pending shapes, keeps fallback shapes, and uses stable date/path/id keysets. Source paths and image bytes are not stored. Existing catalog round-trip, migration, settings, pagination, and offline-retention tests pass.

## Concerns

`AssetShapeUpdate` now has a required `shape_status` field, so downstream workspace callers outside `photo-catalog` will need to populate it when they are updated in the next task. UTC normalization remains in Task 2 as directed.

## TDD evidence

RED:

`cargo test -p photo-catalog --test wall_query`

Failed to compile because `ShapeStatus`, `WallOrder`, `Catalog::wall_page`, `NewAsset::folder_group_id`, and `AssetShapeUpdate::shape_status` did not exist. This was the expected missing-API failure.

GREEN:

`cargo test -p photo-catalog --test wall_query`

Passed 3 tests.

`cargo test -p photo-catalog`

Passed all catalog unit, integration, wall-query, settings, and doc tests.

## Fix round 1

Changes made:

- Fixed provisional cursor pagination to bind the correct `LIMIT` parameter.
- Fixed `has_completed_generation` to read nullable `completed_at` safely.
- Added `ShapeStatus::Pending` so asset reads preserve the migration state instead of reporting fallback.
- Added provisional multi-page coverage, equal-date path tie coverage in both directions, incomplete-generation coverage, and pending asset round-trip coverage.

Covering tests: `crates/catalog/tests/wall_query.rs`.

RED:

`cargo test -p photo-catalog --test wall_query`

Failed to compile because `ShapeStatus::Pending` was missing. After adding that test API, the focused run failed at runtime for incomplete generations with `Sqlite(InvalidColumnType(0, "completed_at", Null))`, confirming the nullable-read defect. The provisional pagination test also exercised the broken cursor branch before its fix.

GREEN:

`cargo test -p photo-catalog --test wall_query`

Passed all 6 focused wall and generation tests.

`cargo test -p photo-catalog`

Passed the full package suite: catalog unit tests, 5 round-trip tests, 2 settings tests, 6 wall-query tests, and doc tests.
