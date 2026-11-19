-- 0008_accounts.sql — website user accounts + account↔machine binding.
--
-- The account layer on top of the existing device enrollment (device_codes /
-- userCode / approve flow, migration 0001): email + scrypt password_hash
-- (self-describing "scrypt$N$r$p$<salt-b64>$<hash-b64>" string — no raw
-- secret is ever stored), 30-day server sessions behind the HttpOnly
-- msw_session cookie, and one row per (user, installation) so the website
-- can show "your machines".
--
-- Same privacy rules as 0001 (asserted by tests/schema-ddl.test.ts): no
-- forbidden column names, no unbounded text, every timestamp timestamptz
-- and named *_at. Session expiry is row-aging on expires_at plus
-- write-time pruning in the store — no timers (ADR-001).

-- Website account users. Email is stored lowercased and compared
-- case-insensitively by that normalization.
CREATE TABLE account_users (
  user_id       varchar(64)  PRIMARY KEY,
  email         varchar(254) NOT NULL UNIQUE,
  password_hash varchar(255) NOT NULL,
  display_name  varchar(64)  NOT NULL,
  created_at    timestamptz  NOT NULL DEFAULT now()
);

-- Server-side sessions for account users (the msw_session cookie holds the
-- opaque token). Same shape as `sessions` (0001) but 30-day TTL and keyed by
-- user_id; pruned at write time keeping one expired tombstone per user.
CREATE TABLE user_sessions (
  token      varchar(128) PRIMARY KEY,
  user_id    varchar(64)  NOT NULL REFERENCES account_users (user_id),
  created_at timestamptz  NOT NULL DEFAULT now(),
  expires_at timestamptz  NOT NULL
);
CREATE INDEX user_sessions_user_idx ON user_sessions (user_id);

-- Account↔machine binding. No 'pending' status: dashboard approval creates
-- the row directly as 'active' (the pending phase lives in device_codes).
-- The partial unique index enforces one ACTIVE owner per installation; a
-- revoked row stays as history and can be re-activated only by its own user.
CREATE TABLE account_machines (
  user_id         varchar(64)  NOT NULL REFERENCES account_users (user_id),
  installation_id varchar(64)  NOT NULL REFERENCES installations (installation_id),
  label           varchar(64)  NOT NULL,
  status          varchar(16)  NOT NULL CHECK (status IN ('active', 'revoked')),
  added_at        timestamptz  NOT NULL DEFAULT now(),
  approved_at     timestamptz,
  PRIMARY KEY (user_id, installation_id)
);
CREATE UNIQUE INDEX account_machines_active_installation
  ON account_machines (installation_id) WHERE status = 'active';
