-- 0001_init.sql — ModelSwarm tracker schema (Phase B).
--
-- Privacy rules enforced by tests/schema-ddl.test.ts (acceptance A2/A3):
--   * No column name anywhere contains: prompt, completion, messages,
--     conversation, content, history, hf_token, access_token, api_key, secret.
--   * No unbounded text: every string column is varchar(n) or an enumerated
--     small value. jsonb is used only for protocol-defined structured
--     metadata (manifests per catalog/schema-v2.json, lease profile/address
--     lists, notice objects, authorization rosters) — none of which can carry
--     prompts or completions by schema.
--   * Every timestamp column is timestamptz (UTC) and named *_at.
--   * The hosting-challenge prompt is a code constant, not a column.
--
-- Lease expiry is row-aging everywhere: reads compare expires_at against
-- now(); no background worker exists (ADR-001).

-- Hub users (admins identified out-of-band; X-MSP-Admin tokens map here).
CREATE TABLE users (
  user_id       varchar(64)  PRIMARY KEY,
  role          varchar(32)  NOT NULL CHECK (role IN ('catalog_admin', 'ops')),
  created_at    timestamptz  NOT NULL DEFAULT now()
);

-- Installations: one row per enrolled device (msp-v1 §2.1).
CREATE TABLE installations (
  installation_id  varchar(64)  PRIMARY KEY,
  public_key_b58   varchar(64)  NOT NULL UNIQUE,
  created_at       timestamptz  NOT NULL DEFAULT now()
);

-- Key history per installation (rotations keep old rows).
CREATE TABLE peer_keys (
  installation_id  varchar(64)   NOT NULL REFERENCES installations (installation_id),
  public_key_b58   varchar(64)   NOT NULL,
  added_at         timestamptz   NOT NULL DEFAULT now(),
  PRIMARY KEY (installation_id, added_at)
);

-- Catalog profiles (schema v2, ADR-011): manifest is hashed into profile_id;
-- wrapper columns are mutable admin/UI state.
CREATE TABLE model_profiles (
  profile_id    varchar(71)  PRIMARY KEY,           -- 'msp1:' + 64 hex
  manifest      jsonb        NOT NULL,              -- ModelProfileManifest v2
  display_name  varchar(80)  NOT NULL,
  status        varchar(16)  NOT NULL DEFAULT 'candidate'
                CHECK (status IN ('candidate', 'active', 'deprecated')),
  provenance    jsonb        NOT NULL DEFAULT '{}'::jsonb,
  created_at    timestamptz  NOT NULL DEFAULT now()
);

-- Monotonic catalog version (CatalogEnvelope.catalogVersion).
CREATE TABLE catalog_state (
  singleton  boolean      PRIMARY KEY DEFAULT true CHECK (singleton),
  version    bigint       NOT NULL DEFAULT 1
);
INSERT INTO catalog_state (singleton, version) VALUES (true, 1);

-- Per-installation artifact license acknowledgements.
CREATE TABLE license_acceptances (
  installation_id  varchar(64)   NOT NULL REFERENCES installations (installation_id),
  profile_id       varchar(71)   NOT NULL REFERENCES model_profiles (profile_id),
  accepted_at      timestamptz   NOT NULL DEFAULT now(),
  PRIMARY KEY (installation_id, profile_id)
);

-- Peer leases: the unit of liveness. expiry = row aging on expires_at.
CREATE TABLE peer_leases (
  lease_id           varchar(32)   PRIMARY KEY,     -- 128-bit hex
  installation_id    varchar(64)   NOT NULL REFERENCES installations (installation_id),
  peer_id            varchar(64)   NOT NULL,
  profiles           jsonb         NOT NULL,        -- bounded array of msp1: profile ids
  addresses          jsonb         NOT NULL,        -- bounded array of multiaddr strings
  max_slots          integer       NOT NULL CHECK (max_slots BETWEEN 1 AND 8),
  free_slots         integer       NOT NULL DEFAULT 0,
  queue_ms           integer       NOT NULL DEFAULT 0,
  draining           boolean       NOT NULL DEFAULT false,
  capacity_class     varchar(16)   NOT NULL DEFAULT 'cpu'
                     CHECK (capacity_class IN ('cpu', 'gpu_entry', 'gpu_mid', 'gpu_high')),
  audit_epoch        bigint        NOT NULL DEFAULT 0,
  created_at         timestamptz   NOT NULL DEFAULT now(),
  last_heartbeat_at  timestamptz   NOT NULL DEFAULT now(),
  expires_at         timestamptz   NOT NULL
);
CREATE INDEX peer_leases_expiry_idx ON peer_leases (expires_at);
CREATE INDEX peer_leases_peer_idx ON peer_leases (peer_id, expires_at);

-- Heartbeat observations (metrics only; no payloads).
CREATE TABLE peer_observations (
  observation_id  bigserial     PRIMARY KEY,
  lease_id        varchar(32)   NOT NULL REFERENCES peer_leases (lease_id),
  peer_id         varchar(64)   NOT NULL,
  free_slots      integer       NOT NULL,
  queue_ms        integer       NOT NULL,
  observed_at     timestamptz   NOT NULL DEFAULT now()
);

-- Hosting challenges. The prompt is a fixed code constant; timings are the
-- measured evidence used for verified_capacity (ADR-012).
CREATE TABLE hosting_challenges (
  challenge_id    varchar(32)   PRIMARY KEY,
  lease_id        varchar(32)   NOT NULL REFERENCES peer_leases (lease_id),
  profile_id      varchar(71)   NOT NULL REFERENCES model_profiles (profile_id),
  issued_at       timestamptz   NOT NULL DEFAULT now(),
  deadline_at     timestamptz   NOT NULL,
  completed_at    timestamptz,
  outcome         varchar(16)   NOT NULL DEFAULT 'open'
                  CHECK (outcome IN ('open', 'passed', 'failed')),
  first_token_ms  integer,
  total_ms        integer
);

-- Capability tokens / eligibility leases (ADR-006 + ADR-012). Serialized wire
-- form is base64url(json).base64url(sig); the signature itself is not stored.
CREATE TABLE capability_tokens (
  nonce           varchar(32)   PRIMARY KEY,
  lease_id        varchar(32)   NOT NULL REFERENCES peer_leases (lease_id),
  peer_id         varchar(64)   NOT NULL,
  installation_id varchar(64)   NOT NULL,
  profile_id      varchar(71)   NOT NULL,
  challenge_id    varchar(32)   NOT NULL,
  can_host        boolean       NOT NULL,
  can_consume     boolean       NOT NULL,
  slots           integer       NOT NULL CHECK (slots BETWEEN 0 AND 8),
  capacity_class  varchar(16)   NOT NULL,
  audit_epoch     bigint        NOT NULL,
  issued_at       timestamptz   NOT NULL,
  expires_at      timestamptz   NOT NULL            -- capped: lease expiry + 60 s
);
CREATE INDEX capability_tokens_expiry_idx ON capability_tokens (expires_at);

-- Job receipts: digest + outcome only (msp-v1 §7).
CREATE TABLE job_receipts (
  receipt_digest    varchar(80)  PRIMARY KEY,
  outcome           varchar(16)  NOT NULL,
  installation_id   varchar(64)  NOT NULL,
  received_at       timestamptz  NOT NULL DEFAULT now()
);

-- Revoked/blocked peers.
CREATE TABLE blocked_peers (
  peer_id     varchar(64)  PRIMARY KEY,
  reason      varchar(120) NOT NULL DEFAULT 'admin',
  blocked_at  timestamptz  NOT NULL DEFAULT now()
);

-- Desktop release channels (ops metadata; no binaries stored here).
CREATE TABLE release_channels (
  channel        varchar(32)  PRIMARY KEY CHECK (channel IN ('stable', 'beta', 'nightly')),
  version        varchar(32)  NOT NULL,
  notes_url      varchar(300),
  published_at   timestamptz  NOT NULL DEFAULT now()
);

-- Envelope nonce cache: single-use per installation inside the replay window
-- (row-aging eviction by the reader; no timers).
CREATE TABLE nonces (
  installation_id  varchar(64)   NOT NULL,
  nonce            varchar(128)  NOT NULL,
  seen_at          timestamptz   NOT NULL DEFAULT now(),
  PRIMARY KEY (installation_id, nonce)
);

-- Persisted rate-limit counters (observability; the live limiter is the
-- in-memory fixed-window buckets per msp-v1 §3.4).
CREATE TABLE rate_counters (
  bucket_key       varchar(160)  NOT NULL,
  window_started_at timestamptz  NOT NULL,
  hits             integer       NOT NULL DEFAULT 0,
  PRIMARY KEY (bucket_key, window_started_at)
);

-- Rendezvous mailbox: opaque SDP-like blobs (<= 4 KiB), deleted on read.
CREATE TABLE rendezvous (
  id            bigserial    PRIMARY KEY,
  to_peer_id    varchar(64)  NOT NULL,
  from_peer_id  varchar(64)  NOT NULL,
  kind          varchar(8)   NOT NULL CHECK (kind IN ('offer', 'answer')),
  payload       varchar(4096) NOT NULL,
  received_at   timestamptz  NOT NULL DEFAULT now()
);
CREATE INDEX rendezvous_to_idx ON rendezvous (to_peer_id);

-- Device enrollment (msp-v1 §3.2). user_code is the short human check code.
CREATE TABLE device_codes (
  device_code      varchar(64)  PRIMARY KEY,
  installation_id  varchar(64)  NOT NULL REFERENCES installations (installation_id),
  user_code        varchar(16)  NOT NULL,
  verify_url       varchar(300) NOT NULL,
  approved         boolean      NOT NULL DEFAULT false,
  created_at       timestamptz  NOT NULL DEFAULT now(),
  expires_at       timestamptz  NOT NULL
);

-- Sessions issued at device/complete: 24 h opaque tokens (X-MSP-Session).
CREATE TABLE sessions (
  token            varchar(128) PRIMARY KEY,
  installation_id  varchar(64)  NOT NULL REFERENCES installations (installation_id),
  created_at       timestamptz  NOT NULL DEFAULT now(),
  expires_at       timestamptz  NOT NULL
);

-- Revocation-propagation notice queues, drained on heartbeat (delete-on-read).
CREATE TABLE peer_notices (
  notice_id    bigserial    PRIMARY KEY,
  lease_id     varchar(32)  NOT NULL REFERENCES peer_leases (lease_id),
  notice       jsonb        NOT NULL,   -- msp-v1 §3.3 Notice objects (bounded)
  created_at   timestamptz  NOT NULL DEFAULT now()
);

-- Authoritative per-peer audit epoch (ADR-012). Leases pin the epoch at
-- registration; a sweep bumps this table only, so leases referencing an
-- older epoch become ineligible without any per-token lookup.
CREATE TABLE peer_audit_state (
  peer_id      varchar(64)  PRIMARY KEY,
  audit_epoch  bigint       NOT NULL DEFAULT 0,
  updated_at   timestamptz  NOT NULL DEFAULT now()
);

-- Cooperative session authorizations (Phase B metadata-only endpoint).
CREATE TABLE session_authorizations (
  session_id    varchar(32)  PRIMARY KEY,
  profile_id    varchar(71)  NOT NULL REFERENCES model_profiles (profile_id),
  mode          varchar(24)  NOT NULL,
  peer_ids      jsonb        NOT NULL,  -- bounded roster (<= 16 peer ids)
  created_at    timestamptz  NOT NULL DEFAULT now(),
  expires_at    timestamptz  NOT NULL
);
