use rusqlite::Connection;

use crate::error::StoreError;
use crate::migrate::{apply_migrations, migrate, Migration};

fn open_migrated() -> Connection {
    let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
    migrate(&mut conn).expect("bundled migration applies");
    conn
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .expect("query sqlite_master");
    count == 1
}

fn view_exists(conn: &Connection, name: &str) -> bool {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .expect("query sqlite_master");
    count == 1
}

fn insert_run(conn: &Connection, source_id: &str, finished_at: &str, value_as_of: &str) -> i64 {
    conn.execute(
        "INSERT INTO load_runs (source_id, status, finished_at, value_as_of)
         VALUES (?1, 'success', ?2, ?3)",
        [source_id, finished_at, value_as_of],
    )
    .expect("insert load_run");
    conn.last_insert_rowid()
}

fn insert_entity(conn: &Connection, load_run_id: i64, taxkey: &str, value_as_of: &str) {
    conn.execute(
        "INSERT INTO entities (load_run_id, taxkey, value_as_of)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![load_run_id, taxkey, value_as_of],
    )
    .expect("insert entity");
}

#[test]
fn migration_applies_clean() {
    let conn = open_migrated();
    assert!(table_exists(&conn, "load_runs"));
    assert!(table_exists(&conn, "entities"));
    assert!(table_exists(&conn, "muni_lookup"));
    assert!(table_exists(&conn, "term_glossary"));
    assert!(table_exists(&conn, "vertical_catalog"));
    assert!(table_exists(&conn, "signal_catalog"));
    assert!(table_exists(&conn, "signal_direction"));
    assert!(table_exists(&conn, "detections"));
    assert!(table_exists(&conn, "current_parcels"));
    assert!(table_exists(&conn, "series_observations"));
    assert!(table_exists(&conn, "series_catalog"));
    assert!(table_exists(&conn, "assignment_records"));
    assert!(table_exists(&conn, "assignment_assignees"));
    assert!(table_exists(&conn, "assignment_assignors"));
    assert!(table_exists(&conn, "assignment_properties"));
    assert!(table_exists(&conn, "outcomes"));
    assert!(table_exists(&conn, "liquor_snapshots"));
    assert!(table_exists(&conn, "liquor_licences"));
    assert!(table_exists(&conn, "lira_applications"));
    assert!(table_exists(&conn, "lira_pdf_extractions"));
    assert!(table_exists(&conn, "lira_pdf_extraction_fields"));
    assert!(table_exists(&conn, "schema_migrations"));
    assert!(view_exists(&conn, "current_entities"));
    assert!(view_exists(&conn, "lira_non_renewal_applications"));
}

#[test]
fn muni_lookup_has_county_scoped_primary_key() {
    let conn = open_migrated();
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'muni_lookup'",
            [],
            |row| row.get(0),
        )
        .expect("muni_lookup sql");
    assert!(
        sql.contains("county_fips") && sql.contains("PRIMARY KEY"),
        "expected county-scoped PK, got {sql}"
    );
}

#[test]
fn applying_twice_is_a_noop() {
    let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
    migrate(&mut conn).expect("first apply");
    migrate(&mut conn).expect("second apply is a no-op");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0))
        .expect("count schema_migrations");
    assert_eq!(count, 21);
    assert!(table_exists(&conn, "entities"));
    assert!(view_exists(&conn, "current_entities"));
}

#[test]
fn changed_migration_file_is_refused() {
    let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
    let original = [Migration {
        filename: "001_initial.sql",
        sql: "CREATE TABLE load_runs (id INTEGER PRIMARY KEY, source_id TEXT NOT NULL, status TEXT NOT NULL);",
    }];
    apply_migrations(&mut conn, &original).expect("original applies");
    let changed = [Migration {
        filename: "001_initial.sql",
        sql: "CREATE TABLE load_runs (id INTEGER PRIMARY KEY, source_id TEXT NOT NULL, status TEXT NOT NULL, extra TEXT);",
    }];
    match apply_migrations(&mut conn, &changed) {
        Err(StoreError::ChecksumMismatch { filename }) => {
            assert_eq!(filename, "001_initial.sql");
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

#[test]
fn second_load_run_inserts_rather_than_updates() {
    let conn = open_migrated();
    let run_one = insert_run(&conn, "src-waukesha-parcels", "2025-12-31T00:00:00Z", "2025-12-31");
    insert_entity(&conn, run_one, "TK-1", "2025-12-31");
    let run_two = insert_run(&conn, "src-waukesha-parcels", "2026-08-10T00:00:00Z", "2026-08-10");
    insert_entity(&conn, run_two, "TK-1", "2026-08-10");
    let entity_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE taxkey = 'TK-1'",
            [],
            |row| row.get(0),
        )
        .expect("count entities");
    assert_eq!(entity_rows, 2);
    let run_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM load_runs", [], |row| row.get(0))
        .expect("count load_runs");
    assert_eq!(run_rows, 2);
}

#[test]
fn current_entities_one_row_per_taxkey_across_two_runs() {
    let conn = open_migrated();
    let run_one = insert_run(&conn, "src-waukesha-parcels", "2025-12-31T00:00:00Z", "2025-12-31");
    insert_entity(&conn, run_one, "TK-1", "2025-12-31");
    insert_entity(&conn, run_one, "TK-2", "2025-12-31");
    let run_two = insert_run(&conn, "src-waukesha-parcels", "2026-08-10T00:00:00Z", "2026-08-10");
    insert_entity(&conn, run_two, "TK-1", "2026-08-10");
    insert_entity(&conn, run_two, "TK-2", "2026-08-10");

    let join_matches: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities e INNER JOIN load_runs r ON r.id = e.load_run_id",
            [],
            |row| row.get(0),
        )
        .expect("join match count");
    println!("entities-load_runs join match count: {join_matches}");
    assert_eq!(join_matches, 4);

    let current_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM current_entities", [], |row| row.get(0))
        .expect("count current_entities");
    assert_eq!(current_count, 2);

    let tk1_as_of: String = conn
        .query_row(
            "SELECT value_as_of FROM current_entities WHERE taxkey = 'TK-1'",
            [],
            |row| row.get(0),
        )
        .expect("TK-1 current value_as_of");
    assert_eq!(tk1_as_of, "2026-08-10");
}

#[test]
fn absent_from_newer_run_resolves_with_original_value_as_of() {
    let conn = open_migrated();
    let run_one = insert_run(&conn, "src-waukesha-parcels", "2025-12-31T00:00:00Z", "2025-12-31");
    insert_entity(&conn, run_one, "TK-1", "2025-12-31");
    insert_entity(&conn, run_one, "TK-2", "2025-12-31");
    let run_two = insert_run(&conn, "src-waukesha-parcels", "2026-08-10T00:00:00Z", "2026-08-10");
    insert_entity(&conn, run_two, "TK-1", "2026-08-10");

    let join_matches: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities e INNER JOIN load_runs r ON r.id = e.load_run_id",
            [],
            |row| row.get(0),
        )
        .expect("join match count");
    println!("entities-load_runs join match count: {join_matches}");
    assert_eq!(join_matches, 3);

    let current_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM current_entities", [], |row| row.get(0))
        .expect("count current_entities");
    assert_eq!(current_count, 2);

    let tk2_as_of: String = conn
        .query_row(
            "SELECT value_as_of FROM current_entities WHERE taxkey = 'TK-2'",
            [],
            |row| row.get(0),
        )
        .expect("TK-2 current value_as_of");
    assert_eq!(tk2_as_of, "2025-12-31");

    let tk1_as_of: String = conn
        .query_row(
            "SELECT value_as_of FROM current_entities WHERE taxkey = 'TK-1'",
            [],
            |row| row.get(0),
        )
        .expect("TK-1 current value_as_of");
    assert_eq!(tk1_as_of, "2026-08-10");
}
