ALTER TABLE app_state
  ADD COLUMN gallery_scope TEXT NOT NULL DEFAULT 'include_subfolders'
  CHECK(gallery_scope IN ('current_folder', 'include_subfolders'));

ALTER TABLE assets ADD COLUMN relative_parent_key BLOB;

PRAGMA user_version = 8;
