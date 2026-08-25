CREATE INDEX warnings_asset_occurred
  ON warnings(asset_id, occurred_at DESC);

CREATE INDEX assets_group_provisional_wall
  ON assets(folder_group_id, provisional_order, id)
  WHERE shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL;

CREATE INDEX assets_group_capture_wall
  ON assets(folder_group_id, captured_at_utc, display_path, id)
  WHERE shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL
    AND captured_at_utc IS NOT NULL;

CREATE INDEX assets_group_capture_desc_wall
  ON assets(folder_group_id, captured_at_utc DESC, display_path, id)
  WHERE shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL
    AND captured_at_utc IS NOT NULL;

PRAGMA user_version = 6;
