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
