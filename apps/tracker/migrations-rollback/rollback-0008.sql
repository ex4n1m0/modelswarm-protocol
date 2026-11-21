-- Rollback for migration 0008 (accounts). All three tables are new and
-- nothing else references them; drop children before parents. Running
-- this deletes every account, browser session, and machine binding —
-- the account layer has no other state.
DROP TABLE IF EXISTS account_machines;
DROP TABLE IF EXISTS user_sessions;
DROP TABLE IF EXISTS account_users;
