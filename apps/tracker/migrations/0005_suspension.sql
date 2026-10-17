-- ADR-012 suspension wiring (gate D6, plan edge #5): slots-to-zero becomes
-- representable, and suspension lives PEER-keyed so a fresh registration
-- cannot escape it. Fully additive except the CHECK swap; every new column
-- follows the A2/A3 rules (timestamptz *_at, bounded varchar, no forbidden
-- names).

-- 1) peer_leases.max_slots: 0 becomes storable (0 = suspended/clamped;
--    the peer_suspensions row is the authority, the lease row carries the
--    enforced value). Every issuance path already treats <1 as ineligible.
ALTER TABLE peer_leases DROP CONSTRAINT peer_leases_max_slots_check;
ALTER TABLE peer_leases ADD CONSTRAINT peer_leases_max_slots_check
  CHECK (max_slots BETWEEN 0 AND 8);

-- 2) Peer-keyed suspension state (survives re-registration: the register
--    route clamps maxSlots to 0 while a live suspension row exists and
--    restores nothing until the backoff window ages out).
CREATE TABLE peer_suspensions (
  peer_id          varchar(64)  PRIMARY KEY,
  until_at         timestamptz  NOT NULL,
  reason           varchar(32)  NOT NULL,
  suspension_count integer      NOT NULL DEFAULT 1,
  prev_max_slots   integer      NOT NULL CHECK (prev_max_slots BETWEEN 1 AND 8),
  suspended_at     timestamptz  NOT NULL DEFAULT now()
);
CREATE INDEX peer_suspensions_until_idx ON peer_suspensions (until_at);

-- 3) Challenge failure forensics: 'failed' rows say WHICH class of failure
--    ('timeout' = ignored past deadline, 'invalid' = implausible/wrong
--    completion) and when (deadline_at for timeouts, completed_at for
--    invalid). The suspension policy reads these by peer across leases.
ALTER TABLE hosting_challenges ADD COLUMN failure_class varchar(16);
CREATE INDEX hosting_challenges_lease_outcome_idx
  ON hosting_challenges (lease_id, outcome);
