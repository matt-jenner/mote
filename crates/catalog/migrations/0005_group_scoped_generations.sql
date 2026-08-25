ALTER TABLE scan_generations
  ADD COLUMN folder_group_id BLOB REFERENCES folder_groups(id) ON DELETE CASCADE;

-- A v4 scan generation covered the whole library. Preserve its completed
-- state for every folder group represented by the generation's assets. Group
-- generations need distinct keys because the v5 primary key remains
-- (library_id, generation), so allocate new generation numbers above the
-- library's legacy range while retaining the original timestamps/source.
WITH legacy_group_generations AS (
  SELECT
    sg.library_id,
    a.folder_group_id,
    sg.started_at,
    sg.completed_at,
    sg.source_was_online,
    ROW_NUMBER() OVER (
      PARTITION BY sg.library_id, a.folder_group_id
      ORDER BY sg.generation DESC
    ) AS group_rank
  FROM scan_generations AS sg
  JOIN assets AS a
    ON a.library_id = sg.library_id
   AND a.last_seen_generation = sg.generation
  WHERE sg.folder_group_id IS NULL
    AND sg.completed_at IS NOT NULL
    AND a.folder_group_id IS NOT NULL
  GROUP BY
    sg.library_id,
    a.folder_group_id,
    sg.generation,
    sg.started_at,
    sg.completed_at,
    sg.source_was_online
),
groups_to_backfill AS (
  SELECT library_id, folder_group_id, started_at, completed_at, source_was_online
  FROM legacy_group_generations
  WHERE group_rank = 1
),
numbered_groups AS (
  SELECT
    library_id,
    folder_group_id,
    started_at,
    completed_at,
    source_was_online,
    COALESCE(
      (
        SELECT MAX(existing.generation)
        FROM scan_generations AS existing
        WHERE existing.library_id = groups_to_backfill.library_id
      ),
      0
    ) + ROW_NUMBER() OVER (
      PARTITION BY library_id
      ORDER BY folder_group_id
    ) AS generation
  FROM groups_to_backfill
)
INSERT INTO scan_generations
  (library_id, folder_group_id, generation, started_at, completed_at, source_was_online)
SELECT library_id, folder_group_id, generation, started_at, completed_at, source_was_online
FROM numbered_groups;

CREATE INDEX scan_generations_group
  ON scan_generations(library_id, folder_group_id, generation, completed_at);

PRAGMA user_version = 5;
