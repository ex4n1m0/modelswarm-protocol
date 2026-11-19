-- 0011_machine_signaling.sql — account directory groundwork (ADR-013 P2).
--
-- signaling_device_id maps an installation to its rd15 signaling mailbox id
-- (16 lowercase hex chars) so an account-scoped machine list can be
-- connectable in-app without a pairing-code exchange. The machine reports it
-- in the signed heartbeat (latest-wins; NULL while the machine has sharing
-- off or has never heartbeated a mapping — those rows list as online but
-- not controllable). Deliberately NOT unique: all rows sharing an id sit in
-- the SAME account (the partial unique index on installation keeps one
-- active owner), so a collision can only shadow one of the owner's own
-- machines, never cross accounts.
--
-- label drops its NOT NULL: a binding created through the claim handshake
-- may carry no label, and the heartbeat auto-populates it from the
-- machine's display name only while it is NULL (an owner rename always
-- wins). listAccountMachines surfaces NULL to the dashboard, which renders
-- a placeholder.
ALTER TABLE account_machines
  ADD COLUMN signaling_device_id varchar(16);

ALTER TABLE account_machines
  ALTER COLUMN label DROP NOT NULL;
