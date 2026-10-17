-- Rollback for migration 0006 (D6 pairing). Restores the original
-- unbounded TEXT types of 0002/0003/0004 — a widening, so no data risk;
-- run only after deleting the 0006 row from schema_migrations.

ALTER TABLE download_counts ALTER COLUMN file TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN artifact_sha256 TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN hf_repo TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN hf_revision TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN artifact_path TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN quant_method TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN display_name TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN note TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN requested_from TYPE TEXT;
ALTER TABLE model_requests ALTER COLUMN resolution TYPE TEXT;
ALTER TABLE granted_accounting ALTER COLUMN receipt_digest TYPE TEXT;
ALTER TABLE granted_accounting ALTER COLUMN peer_id TYPE TEXT;
ALTER TABLE granted_accounting ALTER COLUMN installation_id TYPE TEXT;
ALTER TABLE granted_accounting ALTER COLUMN profile_id TYPE TEXT;
