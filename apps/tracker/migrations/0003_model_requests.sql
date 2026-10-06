-- ADR-023: community model-request queue (public POST /catalog/requests).
-- Stores only artifact pointers the resolver needs — repo, pinned revision,
-- filename, artifact sha256/bytes, quant hint. Content-blind by schema:
-- no field can carry prompts or completions. Dedupes on artifact_sha256
-- (one request per exact artifact); owner resolves via the admin routes.
CREATE TABLE IF NOT EXISTS model_requests (
    id              BIGSERIAL PRIMARY KEY,
    artifact_sha256 TEXT        NOT NULL UNIQUE,
    hf_repo         TEXT        NOT NULL,
    hf_revision     TEXT        NOT NULL,
    artifact_path   TEXT        NOT NULL,
    artifact_bytes  BIGINT      NOT NULL,
    quant_method    TEXT        NOT NULL,
    quant_bits      INT         NOT NULL,
    display_name    TEXT        NOT NULL,
    note            TEXT        NOT NULL DEFAULT '',
    requested_from  TEXT        NOT NULL DEFAULT 'unknown',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ,
    resolution      TEXT        NOT NULL DEFAULT ''   -- '' open | promoted | rejected
);
