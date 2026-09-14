CREATE TABLE folder_group_preview_counts_by_media (
  folder_group_id BLOB NOT NULL REFERENCES folder_groups(id) ON DELETE CASCADE,
  media_kind TEXT NOT NULL,
  wall_ready INTEGER NOT NULL DEFAULT 0 CHECK(wall_ready >= 0),
  screen_ready INTEGER NOT NULL DEFAULT 0 CHECK(screen_ready >= 0),
  direct_wall_ready INTEGER NOT NULL DEFAULT 0 CHECK(direct_wall_ready >= 0),
  direct_screen_ready INTEGER NOT NULL DEFAULT 0 CHECK(direct_screen_ready >= 0),
  PRIMARY KEY(folder_group_id, media_kind)
) WITHOUT ROWID;

INSERT INTO folder_group_preview_counts_by_media(
  folder_group_id,
  media_kind,
  wall_ready,
  screen_ready,
  direct_wall_ready,
  direct_screen_ready
)
SELECT
  membership.folder_group_id,
  asset.media_kind,
  SUM(EXISTS(
    SELECT 1 FROM derivatives derivative
    WHERE derivative.asset_id = asset.id AND derivative.kind = 'wall_thumbnail'
  )),
  SUM(EXISTS(
    SELECT 1 FROM derivatives derivative
    WHERE derivative.asset_id = asset.id AND derivative.kind = 'screen_preview'
  )),
  SUM(
    asset.relative_parent_key = folder_group.relative_path_key
    AND EXISTS(
      SELECT 1 FROM derivatives derivative
      WHERE derivative.asset_id = asset.id AND derivative.kind = 'wall_thumbnail'
    )
  ),
  SUM(
    asset.relative_parent_key = folder_group.relative_path_key
    AND EXISTS(
      SELECT 1 FROM derivatives derivative
      WHERE derivative.asset_id = asset.id AND derivative.kind = 'screen_preview'
    )
  )
FROM folder_group_assets membership
JOIN assets asset ON asset.id = membership.asset_id
JOIN folder_groups folder_group ON folder_group.id = membership.folder_group_id
GROUP BY membership.folder_group_id, asset.media_kind;

CREATE TRIGGER media_preview_counts_after_membership_insert
AFTER INSERT ON folder_group_assets
BEGIN
  INSERT INTO folder_group_preview_counts_by_media(
    folder_group_id,
    media_kind,
    wall_ready,
    screen_ready,
    direct_wall_ready,
    direct_screen_ready
  )
  SELECT
    NEW.folder_group_id,
    asset.media_kind,
    EXISTS(
      SELECT 1 FROM derivatives WHERE asset_id = NEW.asset_id AND kind = 'wall_thumbnail'
    ),
    EXISTS(
      SELECT 1 FROM derivatives WHERE asset_id = NEW.asset_id AND kind = 'screen_preview'
    ),
    asset.relative_parent_key = folder_group.relative_path_key
      AND EXISTS(
        SELECT 1 FROM derivatives WHERE asset_id = NEW.asset_id AND kind = 'wall_thumbnail'
      ),
    asset.relative_parent_key = folder_group.relative_path_key
      AND EXISTS(
        SELECT 1 FROM derivatives WHERE asset_id = NEW.asset_id AND kind = 'screen_preview'
      )
  FROM assets asset
  JOIN folder_groups folder_group ON folder_group.id = NEW.folder_group_id
  WHERE asset.id = NEW.asset_id
  ON CONFLICT(folder_group_id, media_kind) DO UPDATE SET
    wall_ready = wall_ready + excluded.wall_ready,
    screen_ready = screen_ready + excluded.screen_ready,
    direct_wall_ready = direct_wall_ready + excluded.direct_wall_ready,
    direct_screen_ready = direct_screen_ready + excluded.direct_screen_ready;
END;

CREATE TRIGGER media_preview_counts_before_membership_delete
BEFORE DELETE ON folder_group_assets
BEGIN
  UPDATE folder_group_preview_counts_by_media
  SET wall_ready = wall_ready - EXISTS(
        SELECT 1 FROM derivatives WHERE asset_id = OLD.asset_id AND kind = 'wall_thumbnail'
      ),
      screen_ready = screen_ready - EXISTS(
        SELECT 1 FROM derivatives WHERE asset_id = OLD.asset_id AND kind = 'screen_preview'
      ),
      direct_wall_ready = direct_wall_ready - (
        EXISTS(
          SELECT 1 FROM derivatives WHERE asset_id = OLD.asset_id AND kind = 'wall_thumbnail'
        )
        AND EXISTS(
          SELECT 1
          FROM assets asset
          JOIN folder_groups folder_group ON folder_group.id = OLD.folder_group_id
          WHERE asset.id = OLD.asset_id
            AND asset.relative_parent_key = folder_group.relative_path_key
        )
      ),
      direct_screen_ready = direct_screen_ready - (
        EXISTS(
          SELECT 1 FROM derivatives WHERE asset_id = OLD.asset_id AND kind = 'screen_preview'
        )
        AND EXISTS(
          SELECT 1
          FROM assets asset
          JOIN folder_groups folder_group ON folder_group.id = OLD.folder_group_id
          WHERE asset.id = OLD.asset_id
            AND asset.relative_parent_key = folder_group.relative_path_key
        )
      )
  WHERE folder_group_id = OLD.folder_group_id
    AND media_kind = (SELECT media_kind FROM assets WHERE id = OLD.asset_id);
END;

CREATE TRIGGER media_preview_counts_after_derivative_insert
AFTER INSERT ON derivatives
WHEN NEW.kind IN ('wall_thumbnail', 'screen_preview')
 AND NOT EXISTS (
   SELECT 1 FROM derivatives
   WHERE asset_id = NEW.asset_id AND kind = NEW.kind AND id <> NEW.id
 )
BEGIN
  UPDATE folder_group_preview_counts_by_media
  SET wall_ready = wall_ready + (NEW.kind = 'wall_thumbnail'),
      screen_ready = screen_ready + (NEW.kind = 'screen_preview'),
      direct_wall_ready = direct_wall_ready + (
        NEW.kind = 'wall_thumbnail'
        AND EXISTS(
          SELECT 1
          FROM assets asset
          JOIN folder_groups folder_group
            ON folder_group.id = folder_group_preview_counts_by_media.folder_group_id
          WHERE asset.id = NEW.asset_id
            AND asset.relative_parent_key = folder_group.relative_path_key
        )
      ),
      direct_screen_ready = direct_screen_ready + (
        NEW.kind = 'screen_preview'
        AND EXISTS(
          SELECT 1
          FROM assets asset
          JOIN folder_groups folder_group
            ON folder_group.id = folder_group_preview_counts_by_media.folder_group_id
          WHERE asset.id = NEW.asset_id
            AND asset.relative_parent_key = folder_group.relative_path_key
        )
      )
  WHERE folder_group_id IN (
    SELECT folder_group_id FROM folder_group_assets WHERE asset_id = NEW.asset_id
  )
    AND media_kind = (SELECT media_kind FROM assets WHERE id = NEW.asset_id);
END;

CREATE TRIGGER media_preview_counts_after_derivative_delete
AFTER DELETE ON derivatives
WHEN OLD.kind IN ('wall_thumbnail', 'screen_preview')
 AND NOT EXISTS (
   SELECT 1 FROM derivatives WHERE asset_id = OLD.asset_id AND kind = OLD.kind
 )
BEGIN
  UPDATE folder_group_preview_counts_by_media
  SET wall_ready = wall_ready - (OLD.kind = 'wall_thumbnail'),
      screen_ready = screen_ready - (OLD.kind = 'screen_preview'),
      direct_wall_ready = direct_wall_ready - (
        OLD.kind = 'wall_thumbnail'
        AND EXISTS(
          SELECT 1
          FROM assets asset
          JOIN folder_groups folder_group
            ON folder_group.id = folder_group_preview_counts_by_media.folder_group_id
          WHERE asset.id = OLD.asset_id
            AND asset.relative_parent_key = folder_group.relative_path_key
        )
      ),
      direct_screen_ready = direct_screen_ready - (
        OLD.kind = 'screen_preview'
        AND EXISTS(
          SELECT 1
          FROM assets asset
          JOIN folder_groups folder_group
            ON folder_group.id = folder_group_preview_counts_by_media.folder_group_id
          WHERE asset.id = OLD.asset_id
            AND asset.relative_parent_key = folder_group.relative_path_key
        )
      )
  WHERE folder_group_id IN (
    SELECT folder_group_id FROM folder_group_assets WHERE asset_id = OLD.asset_id
  )
    AND media_kind = (SELECT media_kind FROM assets WHERE id = OLD.asset_id);
END;

CREATE TRIGGER media_preview_counts_before_asset_projection_update
BEFORE UPDATE OF media_kind, relative_parent_key ON assets
WHEN OLD.media_kind <> NEW.media_kind OR OLD.relative_parent_key <> NEW.relative_parent_key
BEGIN
  UPDATE folder_group_preview_counts_by_media
  SET wall_ready = wall_ready - EXISTS(
        SELECT 1 FROM derivatives WHERE asset_id = OLD.id AND kind = 'wall_thumbnail'
      ),
      screen_ready = screen_ready - EXISTS(
        SELECT 1 FROM derivatives WHERE asset_id = OLD.id AND kind = 'screen_preview'
      ),
      direct_wall_ready = direct_wall_ready - (
        EXISTS(SELECT 1 FROM derivatives WHERE asset_id = OLD.id AND kind = 'wall_thumbnail')
        AND OLD.relative_parent_key = (
          SELECT relative_path_key FROM folder_groups
          WHERE id = folder_group_preview_counts_by_media.folder_group_id
        )
      ),
      direct_screen_ready = direct_screen_ready - (
        EXISTS(SELECT 1 FROM derivatives WHERE asset_id = OLD.id AND kind = 'screen_preview')
        AND OLD.relative_parent_key = (
          SELECT relative_path_key FROM folder_groups
          WHERE id = folder_group_preview_counts_by_media.folder_group_id
        )
      )
  WHERE folder_group_id IN (
    SELECT folder_group_id FROM folder_group_assets WHERE asset_id = OLD.id
  )
    AND media_kind = OLD.media_kind;
END;

CREATE TRIGGER media_preview_counts_after_asset_projection_update
AFTER UPDATE OF media_kind, relative_parent_key ON assets
WHEN OLD.media_kind <> NEW.media_kind OR OLD.relative_parent_key <> NEW.relative_parent_key
BEGIN
  INSERT INTO folder_group_preview_counts_by_media(
    folder_group_id,
    media_kind,
    wall_ready,
    screen_ready,
    direct_wall_ready,
    direct_screen_ready
  )
  SELECT
    membership.folder_group_id,
    NEW.media_kind,
    EXISTS(SELECT 1 FROM derivatives WHERE asset_id = NEW.id AND kind = 'wall_thumbnail'),
    EXISTS(SELECT 1 FROM derivatives WHERE asset_id = NEW.id AND kind = 'screen_preview'),
    NEW.relative_parent_key = folder_group.relative_path_key
      AND EXISTS(SELECT 1 FROM derivatives WHERE asset_id = NEW.id AND kind = 'wall_thumbnail'),
    NEW.relative_parent_key = folder_group.relative_path_key
      AND EXISTS(SELECT 1 FROM derivatives WHERE asset_id = NEW.id AND kind = 'screen_preview')
  FROM folder_group_assets membership
  JOIN folder_groups folder_group ON folder_group.id = membership.folder_group_id
  WHERE membership.asset_id = NEW.id
  ON CONFLICT(folder_group_id, media_kind) DO UPDATE SET
    wall_ready = wall_ready + excluded.wall_ready,
    screen_ready = screen_ready + excluded.screen_ready,
    direct_wall_ready = direct_wall_ready + excluded.direct_wall_ready,
    direct_screen_ready = direct_screen_ready + excluded.direct_screen_ready;
END;

DROP TRIGGER preview_counts_after_folder_group_insert;
DROP TRIGGER preview_counts_after_membership_insert;
DROP TRIGGER preview_counts_before_membership_delete;
DROP TRIGGER preview_counts_after_derivative_insert;
DROP TRIGGER preview_counts_after_derivative_delete;
DROP TRIGGER preview_counts_before_asset_projection_update;
DROP TRIGGER preview_counts_after_asset_projection_update;
DROP TABLE folder_group_preview_counts;

PRAGMA user_version = 14;
