CREATE TABLE photo_picks (
    position INTEGER PRIMARY KEY,
    asset_id BLOB NOT NULL UNIQUE REFERENCES assets(id) ON DELETE CASCADE,
    folder_group_id BLOB NOT NULL REFERENCES folder_groups(id) ON DELETE CASCADE
);

CREATE TABLE photo_pick_preferences (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    last_copy_destination BLOB
);

INSERT INTO photo_pick_preferences (singleton) VALUES (1);
PRAGMA user_version = 13;
