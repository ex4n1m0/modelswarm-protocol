-- Rollback for migration 0009 (machine seen). The column is display-only
-- liveness state; dropping it loses the Online / "Seen …" derivation but no
-- other surface depends on it.
ALTER TABLE account_machines DROP COLUMN IF EXISTS last_seen_at;
