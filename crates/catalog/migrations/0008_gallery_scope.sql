ALTER TABLE app_state
  ADD COLUMN gallery_scope TEXT NOT NULL DEFAULT 'include_subfolders'
  CHECK(gallery_scope IN ('current_folder', 'include_subfolders'));

ALTER TABLE assets ADD COLUMN relative_parent_key BLOB;

CREATE INDEX assets_group_parent_provisional_photo
  ON assets(folder_group_id, relative_parent_key, provisional_order, id)
  WHERE media_kind <> 'video'
    AND shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL;

CREATE INDEX assets_group_parent_capture_photo
  ON assets(folder_group_id, relative_parent_key, captured_at_utc, display_path, id)
  WHERE media_kind <> 'video'
    AND shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL
    AND captured_at_utc IS NOT NULL;

CREATE INDEX assets_group_parent_capture_desc_photo
  ON assets(folder_group_id, relative_parent_key, captured_at_utc DESC, display_path, id)
  WHERE media_kind <> 'video'
    AND shape_status IN ('ready', 'fallback')
    AND width IS NOT NULL
    AND height IS NOT NULL
    AND captured_at_utc IS NOT NULL;

PRAGMA user_version = 8;
