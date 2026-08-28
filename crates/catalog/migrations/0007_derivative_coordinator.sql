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
