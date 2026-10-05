//! Node-local persistence: SQLite via rusqlite, embedded migrations, and
//! typed access for installation state, license acceptances, EWMA
//! observations, and job accounting.
//!
//! Privacy is a structural property of the schema, not a convention: no
//! table has a column for prompts, completions, conversations, tokens, or
//! secrets (AGENTS.md hard constraint 5, `docs/privacy.md`). The
//! integration test in `tests/store.rs` enumerates every table/column via
//! `sqlite_master`/`PRAGMA table_info` and fails on any forbidden name, so a
//! future migration cannot reintroduce one silently.
//!
//! Job rows keep only timings, outcome, and the usage digest of a job
//! receipt (msp-v1 §7) — never request content.

use std::fmt;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Store errors. All database failures surface as [`StoreError::Sqlite`].
#[derive(Debug)]
pub enum StoreError {
    /// A SQLite operation failed.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "sqlite error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Embedded migrations, applied in order by schema version
/// (`PRAGMA user_version`). Append-only: never edit an applied entry, only
/// add `(next_version, sql)` at the end.
const MIGRATIONS: &[(i64, &str)] = &[(
    1,
    r#"
CREATE TABLE IF NOT EXISTS installations (
    id         TEXT PRIMARY KEY,
    pub_key    BLOB NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS license_acceptances (
    installation_id TEXT NOT NULL,
    profile_id      TEXT NOT NULL,
    accepted_at     TEXT NOT NULL,
    PRIMARY KEY (installation_id, profile_id)
);

CREATE TABLE IF NOT EXISTS ewma_observations (
    peer_id    TEXT NOT NULL,
    metric     TEXT NOT NULL,
    value      REAL NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (peer_id, metric)
);

CREATE TABLE IF NOT EXISTS job_accounting (
    request_id   TEXT PRIMARY KEY,
    peer_id      TEXT NOT NULL,
    profile_id   TEXT NOT NULL,
    outcome      TEXT NOT NULL,
    started_at   TEXT NOT NULL,
    ended_at     TEXT NOT NULL,
    usage_digest TEXT NOT NULL
);
"#,
)];

/// The SQLite-backed node-local store.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and applies all
    /// pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    fn migrate(conn: &Connection) -> Result<()> {
        let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        for (version, sql) in MIGRATIONS {
            if *version > current {
                conn.execute_batch(sql)?;
                conn.pragma_update(None, "user_version", *version)?;
            }
        }
        Ok(())
    }

    /// Inserts or refreshes this installation's identity row. `created_at`
    /// is preserved on update.
    pub fn upsert_installation(&self, id: &str, pub_key: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO installations (id, pub_key, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET pub_key = excluded.pub_key",
            rusqlite::params![id, pub_key, now_rfc3339()],
        )?;
        Ok(())
    }

    /// Records the installation's acceptance of a profile's license
    /// (idempotent per installation + profile).
    pub fn record_license(&self, installation_id: &str, profile_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO license_acceptances
                 (installation_id, profile_id, accepted_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![installation_id, profile_id, now_rfc3339()],
        )?;
        Ok(())
    }

    /// Inserts or refreshes the EWMA observation for one peer + metric.
    pub fn upsert_ewma(&self, peer_id: &str, metric: &str, value: f64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO ewma_observations
                 (peer_id, metric, value, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![peer_id, metric, value, now_rfc3339()],
        )?;
        Ok(())
    }

    /// The last stored EWMA value for a peer + metric, if any.
    pub fn get_ewma(&self, peer_id: &str, metric: &str) -> Result<Option<f64>> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM ewma_observations WHERE peer_id = ?1 AND metric = ?2",
                rusqlite::params![peer_id, metric],
                |row| row.get::<_, f64>(0),
            )
            .optional()?;
        Ok(value)
    }

    /// Records one finished job (timings, outcome, usage digest only).
    ///
    /// The argument list mirrors the `job_accounting` columns one-for-one
    /// (msp-v1 §7 receipt fields); a wrapper struct would only re-name them.
    #[allow(clippy::too_many_arguments)]
    pub fn record_job(
        &self,
        request_id: &str,
        peer_id: &str,
        profile_id: &str,
        outcome: &str,
        started_at: &str,
        ended_at: &str,
        usage_digest: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO job_accounting
                 (request_id, peer_id, profile_id, outcome,
                  started_at, ended_at, usage_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                request_id,
                peer_id,
                profile_id,
                outcome,
                started_at,
                ended_at,
                usage_digest
            ],
        )?;
        Ok(())
    }

    /// Total recorded jobs.
    pub fn count_jobs(&self) -> Result<i64> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM job_accounting", [], |row| row.get(0))?;
        Ok(count)
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("formatting the current UTC time as RFC 3339 cannot fail")
}
