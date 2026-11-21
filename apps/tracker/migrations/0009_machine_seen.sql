-- 0009_machine_seen.sql — machine liveness for the account dashboard.
--
-- The signed machine heartbeat (POST /api/v1/account/machines/{id}/heartbeat,
-- MSP1 envelope + x-msp-session like every peer route) touches this column on
-- the installation's ACTIVE account_machines row; the dashboard derives
-- Online / "Seen …" from it at render time. Nullable on purpose: machines
-- bound before this change have never been seen, and a fresh bind resets it
-- (liveness is per-binding, not per-installation history). Row-aging display
-- only — the tracker never schedules work off it (ADR-001, no timers).
ALTER TABLE account_machines ADD COLUMN last_seen_at timestamptz;
