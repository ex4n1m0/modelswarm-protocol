-- ADR-041 swarm balance: the CONSUMED side of the two-sided ledger.
-- Rows are appended at receipt intake (the posting installation IS the
-- requester); decay is read-time (same 7-day half-life as
-- granted_accounting). One row per (installation, receipt) — a re-posted
-- receipt is a no-op. Token counts only; never content (A2).
CREATE TABLE IF NOT EXISTS consumed_accounting (
    installation_id varchar(64)  NOT NULL,
    receipt_digest  varchar(71)  NOT NULL,
    tokens          bigint       NOT NULL,
    consumed_at     timestamptz  NOT NULL DEFAULT now(),
    PRIMARY KEY (installation_id, receipt_digest)
);
CREATE INDEX IF NOT EXISTS consumed_accounting_installation
    ON consumed_accounting (installation_id);
