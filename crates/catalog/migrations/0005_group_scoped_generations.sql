ALTER TABLE scan_generations
  ADD COLUMN folder_group_id BLOB REFERENCES folder_groups(id) ON DELETE CASCADE;

CREATE INDEX scan_generations_group
  ON scan_generations(library_id, folder_group_id, generation, completed_at);

PRAGMA user_version = 5;
