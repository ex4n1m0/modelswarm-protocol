-- Rollback for migration 0005 (gate D6 requires the paired rollback to
-- ship WITH the change). Run manually against the target database only
-- after `DELETE FROM schema_migrations WHERE name = '0005_suspension.sql';`
-- — the forward runner would otherwise re-apply it on the next deploy.
--
-- Order matters: restore any clamped/suspended peers FIRST so no lease row
-- keeps a 0 that the restored CHECK would reject.

-- 1) Un-suspend every peer: restore the remembered slot count on all its
--    live leases, then drop the suspension table.
UPDATE peer_leases pl
   SET max_slots = ps.prev_max_slots
  FROM peer_suspensions ps
 WHERE pl.peer_id = ps.peer_id
   AND pl.max_slots = 0;
DROP TABLE IF EXISTS peer_suspensions;

-- 2) Drop the challenge forensics columns/index.
DROP INDEX IF EXISTS hosting_challenges_lease_outcome_idx;
ALTER TABLE hosting_challenges DROP COLUMN IF EXISTS failure_class;

-- 3) Restore the original CHECK. Any remaining max_slots = 0 with no
--    suspension row (manual tampering) is reset to 1 first — rollback
--    must never fail on dirt the forward path would not have created.
UPDATE peer_leases SET max_slots = 1 WHERE max_slots = 0;
ALTER TABLE peer_leases DROP CONSTRAINT peer_leases_max_slots_check;
ALTER TABLE peer_leases ADD CONSTRAINT peer_leases_max_slots_check
  CHECK (max_slots BETWEEN 1 AND 8);
