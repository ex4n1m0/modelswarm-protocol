-- 0004: granted verified-work accounting (ADR-030 §3/§5). Rows are
-- appended ONLY by intake-validated ReceiptV2 processing (both
-- signatures verified against enrolled keys; self-work and sybil-ring
-- work contribute zero). (receipt_digest, peer_id) is the replay-proof
-- key: a re-posted receipt is a no-op. Decay is read-time (half-life
-- form), never mutated — the raw rows are the auditable evidence base.
CREATE TABLE IF NOT EXISTS granted_accounting (
    receipt_digest    TEXT        NOT NULL,
    peer_id           TEXT        NOT NULL,
    installation_id   TEXT        NOT NULL,
    profile_id        TEXT        NOT NULL,
    endpoint_seconds  BIGINT      NOT NULL,
    verified_tokens   BIGINT      NOT NULL,
    granted_at        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (receipt_digest, peer_id)
);
CREATE INDEX IF NOT EXISTS granted_accounting_peer ON granted_accounting (peer_id);
