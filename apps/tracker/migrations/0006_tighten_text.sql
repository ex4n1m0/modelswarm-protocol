-- A2 hygiene, found by extending schema-ddl.test.ts to ALL migrations
-- (it had parsed 0001 only): 0002/0003/0004 introduced unbounded TEXT
-- columns that violate the no-unbounded-text rule. This migration bounds
-- them to the domain's real shapes; no live row is expected to exceed any
-- limit (sha256 digests are 64 hex, ids are ≤ 71 chars with prefixes).
-- Single-line ALTERs on purpose — the schema test's parser is line-based.
-- (The uppercase TIMESTAMPTZ spellings in 0002-0004 are compliant — the
-- test compares case-insensitively now.)

ALTER TABLE download_counts ALTER COLUMN file TYPE varchar(128);

ALTER TABLE model_requests ALTER COLUMN artifact_sha256 TYPE varchar(71);
ALTER TABLE model_requests ALTER COLUMN hf_repo TYPE varchar(150);
ALTER TABLE model_requests ALTER COLUMN hf_revision TYPE varchar(80);
ALTER TABLE model_requests ALTER COLUMN artifact_path TYPE varchar(240);
ALTER TABLE model_requests ALTER COLUMN quant_method TYPE varchar(24);
ALTER TABLE model_requests ALTER COLUMN display_name TYPE varchar(160);
ALTER TABLE model_requests ALTER COLUMN note TYPE varchar(400);
ALTER TABLE model_requests ALTER COLUMN requested_from TYPE varchar(80);
ALTER TABLE model_requests ALTER COLUMN resolution TYPE varchar(16);

ALTER TABLE granted_accounting ALTER COLUMN receipt_digest TYPE varchar(71);
ALTER TABLE granted_accounting ALTER COLUMN peer_id TYPE varchar(64);
ALTER TABLE granted_accounting ALTER COLUMN installation_id TYPE varchar(64);
ALTER TABLE granted_accounting ALTER COLUMN profile_id TYPE varchar(71);
