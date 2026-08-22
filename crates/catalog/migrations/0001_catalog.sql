CREATE TABLE library_roots (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  kind TEXT NOT NULL CHECK(kind IN ('configured', 'recent')),
  display_name TEXT NOT NULL,
  canonical_root_key BLOB NOT NULL UNIQUE,
  display_path TEXT NOT NULL,
  availability TEXT NOT NULL,
  last_seen_at INTEGER
);

CREATE TABLE folder_groups (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  relative_path_key BLOB NOT NULL,
  display_path TEXT NOT NULL,
  last_viewed_at INTEGER,
  UNIQUE(library_id, relative_path_key)
);

CREATE TABLE library_policies (
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  policy_json TEXT NOT NULL,
  PRIMARY KEY(library_id, position)
);

CREATE TABLE assets (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  folder_group_id BLOB REFERENCES folder_groups(id) ON DELETE SET NULL,
  relative_path_key BLOB NOT NULL,
  display_path TEXT NOT NULL,
  media_kind TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  modified_unix_ns TEXT NOT NULL,
  sidecar_modified_unix_ns TEXT,
  width INTEGER,
  height INTEGER,
  orientation INTEGER,
  representative_rgb INTEGER,
  visible_by_default INTEGER NOT NULL DEFAULT 1,
  availability TEXT NOT NULL,
  captured_at_utc TEXT,
  rating INTEGER CHECK(rating BETWEEN 0 AND 5),
  last_seen_generation INTEGER NOT NULL DEFAULT 0,
  UNIQUE(library_id, relative_path_key)
);

CREATE TABLE metadata_provenance (
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  field_name TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source_path_key BLOB,
  raw_value TEXT NOT NULL,
  chosen INTEGER NOT NULL,
  PRIMARY KEY(asset_id, field_name, source_kind, raw_value)
);

CREATE TABLE asset_keywords (
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  normalized TEXT NOT NULL,
  display_value TEXT NOT NULL,
  hierarchy TEXT,
  PRIMARY KEY(asset_id, normalized, hierarchy)
);

CREATE TABLE scan_generations (
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  started_at INTEGER NOT NULL,
  completed_at INTEGER,
  source_was_online INTEGER NOT NULL,
  PRIMARY KEY(library_id, generation)
);

CREATE TABLE warnings (
  id INTEGER PRIMARY KEY,
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  asset_id BLOB REFERENCES assets(id) ON DELETE CASCADE,
  code TEXT NOT NULL,
  message TEXT NOT NULL,
  occurred_at INTEGER NOT NULL
);

CREATE TABLE derivatives (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  folder_group_id BLOB NOT NULL REFERENCES folder_groups(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  cache_key TEXT NOT NULL UNIQUE,
  relative_cache_path TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  durable INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX assets_library_sort ON assets(library_id, display_path, id);
CREATE INDEX assets_group ON assets(folder_group_id, id);
CREATE INDEX derivatives_group ON derivatives(folder_group_id, durable, created_at);
PRAGMA user_version = 1;
