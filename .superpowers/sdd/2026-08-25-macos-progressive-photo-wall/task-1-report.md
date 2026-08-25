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
