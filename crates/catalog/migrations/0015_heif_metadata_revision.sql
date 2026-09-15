-- Feature-neutral state. Only a completed, online metadata scan may certify
-- a group's HEIF capability. The membership subset bounds startup probes.
ALTER TABLE folder_groups ADD COLUMN heif_metadata_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE scan_generations ADD COLUMN heif_metadata_retry_required INTEGER NOT NULL DEFAULT 0;
CREATE TABLE folder_group_heif_assets (
  folder_group_id BLOB NOT NULL,
  asset_id BLOB NOT NULL,
  PRIMARY KEY(folder_group_id, asset_id),
  FOREIGN KEY(folder_group_id, asset_id)
    REFERENCES folder_group_assets(folder_group_id, asset_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX folder_group_heif_assets_asset ON folder_group_heif_assets(asset_id, folder_group_id);
INSERT INTO folder_group_heif_assets
SELECT membership.folder_group_id, membership.asset_id
FROM folder_group_assets membership JOIN assets asset ON asset.id = membership.asset_id
WHERE asset.media_kind = 'heif';

CREATE TRIGGER heif_metadata_membership_insert
AFTER INSERT ON folder_group_assets
WHEN EXISTS (SELECT 1 FROM assets WHERE id = NEW.asset_id AND media_kind = 'heif')
BEGIN
  INSERT INTO folder_group_heif_assets VALUES (NEW.folder_group_id, NEW.asset_id);
  UPDATE folder_groups SET heif_metadata_revision = 0 WHERE id = NEW.folder_group_id;
END;

CREATE TRIGGER heif_metadata_media_changed
AFTER UPDATE OF media_kind ON assets
WHEN OLD.media_kind <> NEW.media_kind AND (OLD.media_kind = 'heif' OR NEW.media_kind = 'heif')
BEGIN
  UPDATE folder_groups SET heif_metadata_revision = 0
  WHERE id IN (SELECT folder_group_id FROM folder_group_assets WHERE asset_id = NEW.id);
  DELETE FROM folder_group_heif_assets WHERE asset_id = NEW.id;
  INSERT INTO folder_group_heif_assets
  SELECT folder_group_id, asset_id FROM folder_group_assets
  WHERE asset_id = NEW.id AND NEW.media_kind = 'heif';
END;

CREATE TRIGGER heif_metadata_signature_changed
AFTER UPDATE OF size_bytes, modified_unix_ns, sidecar_modified_unix_ns ON assets
WHEN NEW.media_kind = 'heif' AND (
  OLD.size_bytes IS NOT NEW.size_bytes OR
  OLD.modified_unix_ns IS NOT NEW.modified_unix_ns OR
  OLD.sidecar_modified_unix_ns IS NOT NEW.sidecar_modified_unix_ns)
BEGIN
  UPDATE folder_groups SET heif_metadata_revision = 0
  WHERE id IN (SELECT folder_group_id FROM folder_group_heif_assets WHERE asset_id = NEW.id);
END;
PRAGMA user_version = 15;
