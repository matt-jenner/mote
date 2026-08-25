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
