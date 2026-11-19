-- 0010_device_claim.sql — device-side claim confirmation for enrollment
-- (ADR-013 P2 gate (a), Remote repo).
--
-- The account approve flow no longer binds directly: typing the 8-char
-- userCode at the dashboard writes a PENDING claim here, the device's next
-- auth/device/complete poll reports it ("claimed by account X"), and the
-- binding only happens after the device confirms via
-- POST /auth/device/claim-confirm (MSP1-signed). A leaked userCode
-- (screenshot / shoulder-surf) can then at worst dangle a pending claim the
-- device declines — it can no longer silently attach the machine to someone
-- else's account. The admin /verify approve keeps its direct semantics
-- (trusted 256-bit admin token): approved=true with no claim rows touched.
--
-- claimed_label carries the dashboard's optional machine label across the
-- handshake so the confirm-time bind uses what the approver typed. All
-- columns nullable; a row with claimed_by_user set and claim_confirmed_at
-- NULL is a pending claim, and code expiry (15 min) retires it with the row.
-- (FK note: no ON DELETE action — device_codes rows are short-lived (15 min)
-- and pruned at write time, and account_users rows are never deleted today;
-- if user deletion ever lands, prune expired codes first or the FK blocks.)
ALTER TABLE device_codes
  ADD COLUMN claimed_by_user   varchar(64) REFERENCES account_users (user_id),
  ADD COLUMN claimed_label     varchar(64),
  ADD COLUMN claimed_at        timestamptz,
  ADD COLUMN claim_confirmed_at timestamptz;
