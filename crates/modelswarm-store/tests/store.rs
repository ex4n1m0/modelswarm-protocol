//! Integration tests for the node-local store: round-trips on a tempfile
//! database and the structural privacy assertion (no column anywhere may
//! carry prompt-like or secret-like names).

use modelswarm_store::Store;
use rusqlite::Connection;

/// Column-name fragments that must never appear in any table: prompts and
/// completions (AGENTS.md hard constraint 5) and credentials.
const FORBIDDEN_COLUMN_SUBSTRINGS: &[&str] = &[
    "prompt",
    "completion",
    "messages",
    "conversation",
    "content",
    "history",
    "hf_token",
    "access_token",
    "api_key",
    "secret",
];

#[test]
fn open_creates_schema_and_migrations_are_idempotent() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = Store::open(file.path()).unwrap();
    store.upsert_installation("install-1", &[1, 2, 3]).unwrap();
    // Reopening runs migrations again harmlessly (IF NOT EXISTS + versioned).
    let store = Store::open(file.path()).unwrap();
    store.upsert_ewma("peer-1", "ttft_ms", 12.5).unwrap();
    let conn = Connection::open(file.path()).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert!(
        version >= 1,
        "migrations must bump user_version, got {version}"
    );
}

#[test]
fn installation_upsert_round_trips_and_preserves_created_at() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = Store::open(file.path()).unwrap();
    store.upsert_installation("install-1", &[9, 9]).unwrap();
    store.upsert_installation("install-1", &[7, 7]).unwrap();

    let conn = Connection::open(file.path()).unwrap();
    let (pub_key, count): (Vec<u8>, i64) = conn
        .query_row(
            "SELECT pub_key, (SELECT COUNT(*) FROM installations) FROM installations WHERE id = 'install-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(pub_key, vec![7, 7], "conflicting update refreshes the key");
    assert_eq!(count, 1, "upsert must not duplicate rows");
}

#[test]
fn license_records_are_idempotent_per_pair() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = Store::open(file.path()).unwrap();
    store.record_license("install-1", "msp1:aa").unwrap();
    store.record_license("install-1", "msp1:aa").unwrap();
    store.record_license("install-1", "msp1:bb").unwrap();

    let conn = Connection::open(file.path()).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM license_acceptances WHERE installation_id = 'install-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn ewma_round_trips_and_updates() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = Store::open(file.path()).unwrap();
    assert_eq!(store.get_ewma("peer-1", "ttft_ms").unwrap(), None);
    store.upsert_ewma("peer-1", "ttft_ms", 120.5).unwrap();
    assert_eq!(store.get_ewma("peer-1", "ttft_ms").unwrap(), Some(120.5));
    store.upsert_ewma("peer-1", "ttft_ms", 90.25).unwrap();
    assert_eq!(store.get_ewma("peer-1", "ttft_ms").unwrap(), Some(90.25));
    // Different metric, same peer: separate row.
    store.upsert_ewma("peer-1", "itl_ms", 33.0).unwrap();
    assert_eq!(store.get_ewma("peer-1", "itl_ms").unwrap(), Some(33.0));
    assert_eq!(store.get_ewma("peer-1", "ttft_ms").unwrap(), Some(90.25));
    assert_eq!(store.get_ewma("peer-2", "ttft_ms").unwrap(), None);
}

#[test]
fn job_accounting_counts_and_is_idempotent_per_request() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = Store::open(file.path()).unwrap();
    assert_eq!(store.count_jobs().unwrap(), 0);
    for (i, outcome) in ["stop", "error", "cancelled"].iter().enumerate() {
        store
            .record_job(
                &format!("req-{i}"),
                "peer-1",
                "msp1:aa",
                outcome,
                "2026-10-04T12:00:00Z",
                "2026-10-04T12:00:05Z",
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap();
    }
    assert_eq!(store.count_jobs().unwrap(), 3);
    // Re-recording the same request id replaces, not appends.
    store
        .record_job(
            "req-0",
            "peer-1",
            "msp1:aa",
            "stop",
            "2026-10-04T12:00:00Z",
            "2026-10-04T12:00:05Z",
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
    assert_eq!(store.count_jobs().unwrap(), 3);
}

/// Structural privacy gate: enumerate EVERY user table and EVERY column via
/// sqlite_master + PRAGMA table_info and assert no column name contains a
/// forbidden substring. A future migration that adds e.g. a `prompt_text`
/// column fails this test.
#[test]
fn no_table_column_holds_prompt_like_or_secret_like_names() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let _store = Store::open(file.path()).unwrap();

    let conn = Connection::open(file.path()).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(stmt);

    assert!(
        tables.contains(&"installations".to_string())
            && tables.contains(&"license_acceptances".to_string())
            && tables.contains(&"ewma_observations".to_string())
            && tables.contains(&"job_accounting".to_string()),
        "expected the four schema tables, found {tables:?}"
    );

    let mut checked = 0;
    for table in &tables {
        let mut stmt = conn
            .prepare(&format!(
                "PRAGMA table_info(\"{}\")",
                table.replace('"', "\"\"")
            ))
            .unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);
        assert!(!columns.is_empty(), "table {table} has no columns?");
        for column in columns {
            let lower = column.to_lowercase();
            for forbidden in FORBIDDEN_COLUMN_SUBSTRINGS {
                assert!(
                    !lower.contains(forbidden),
                    "privacy violation: {table}.{column} contains forbidden substring {forbidden:?}"
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 15,
        "expected the full column set, checked {checked}"
    );
}
