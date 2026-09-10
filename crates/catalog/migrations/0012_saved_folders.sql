CREATE TABLE saved_folders (
  id BLOB PRIMARY KEY NOT NULL CHECK(length(id) = 16),
  folder_group_id BLOB NOT NULL UNIQUE REFERENCES folder_groups(id) ON DELETE CASCADE,
  custom_label TEXT
);
CREATE TABLE saved_folder_preferences (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  has_opened_folder INTEGER NOT NULL CHECK(has_opened_folder IN (0, 1))
);
INSERT INTO saved_folder_preferences VALUES (1, 0);
PRAGMA user_version = 12;
