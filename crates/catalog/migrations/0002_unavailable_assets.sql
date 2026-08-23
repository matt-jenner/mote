CREATE INDEX assets_unavailable
ON assets(library_id)
WHERE availability <> 'available';

PRAGMA user_version = 2;
