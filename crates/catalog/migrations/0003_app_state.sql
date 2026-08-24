CREATE TABLE app_state (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  appearance TEXT NOT NULL CHECK(appearance IN ('system', 'light', 'dark'))
);

CREATE TABLE active_source_selection (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1)
    REFERENCES app_state(singleton) ON DELETE CASCADE,
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  relative_folder_key BLOB NOT NULL
);

INSERT INTO app_state (singleton, appearance) VALUES (1, 'system');

PRAGMA user_version = 3;
