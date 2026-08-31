ALTER TABLE folder_groups
  ADD COLUMN recovery_requested INTEGER NOT NULL DEFAULT 0
  CHECK(recovery_requested >= 0);

ALTER TABLE folder_groups
  ADD COLUMN recovery_reconciled INTEGER NOT NULL DEFAULT 0
  CHECK(recovery_reconciled >= 0);

ALTER TABLE scan_generations
  ADD COLUMN recovery_token INTEGER NOT NULL DEFAULT 0
  CHECK(recovery_token >= 0);

-- Version 9 did not persist group-local recovery state. Preserve only durable
-- offline evidence: a root-offline library covers even empty groups, while a
-- root-offline membership covers an isolated folder outage.
UPDATE folder_groups
SET recovery_requested = 1
WHERE EXISTS (
        SELECT 1
        FROM library_roots library
        WHERE library.id = folder_groups.library_id
          AND library.availability = 'root_offline'
      )
   OR EXISTS (
        SELECT 1
        FROM folder_group_assets membership
        JOIN assets asset ON asset.id = membership.asset_id
        WHERE membership.folder_group_id = folder_groups.id
          AND asset.library_id = folder_groups.library_id
          AND asset.availability = 'root_offline'
      );

PRAGMA user_version = 10;
