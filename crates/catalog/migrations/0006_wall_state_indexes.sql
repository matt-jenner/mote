CREATE INDEX warnings_asset_occurred
  ON warnings(asset_id, occurred_at DESC);

PRAGMA user_version = 6;
