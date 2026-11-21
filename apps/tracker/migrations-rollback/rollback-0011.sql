-- Rollback for migration 0011 (machine signaling mapping). Loses the
-- in-app connectability mapping; labels that were auto-populated from
-- display names keep their values, and any NULL labels (claim-handshake
-- binds without a label) must be backfilled before the NOT NULL restore.
UPDATE account_machines SET label = 'Machine' WHERE label IS NULL;
ALTER TABLE account_machines
  ALTER COLUMN label SET NOT NULL,
  DROP COLUMN IF EXISTS signaling_device_id;
