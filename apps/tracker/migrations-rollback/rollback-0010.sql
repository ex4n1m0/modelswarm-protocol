-- Rollback for migration 0010 (device claim confirmation). Pending or
-- confirmed claims are handshake state only — the binding itself lives in
-- account_machines — so dropping the columns loses nothing but the claim
-- audit trail.
ALTER TABLE device_codes
  DROP COLUMN IF EXISTS claimed_by_user,
  DROP COLUMN IF EXISTS claimed_label,
  DROP COLUMN IF EXISTS claimed_at,
  DROP COLUMN IF EXISTS claim_confirmed_at;
