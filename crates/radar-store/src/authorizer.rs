use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::Connection;

const RECORD_TABLES: &[&str] = &[
    "entities",
    "load_runs",
    "muni_lookup",
    "detections",
    "series_observations",
    "series_catalog",
    "assignment_records",
    "assignment_assignees",
    "assignment_assignors",
    "assignment_properties",
    "outcomes",
    "liquor_snapshots",
    "liquor_licences",
    "lira_applications",
    "lira_pdf_extractions",
    "lira_pdf_extraction_fields",
];

pub fn install_write_authorizer(conn: &Connection) {
    conn.authorizer(Some(authorize));
}

fn is_record_table(name: &str) -> bool {
    RECORD_TABLES.iter().any(|table| *table == name)
}

// UPDATE and DELETE are denied on record tables, including outcomes.
// current_parcels is exempt because it is a cache of a derivable fact
// and is rebuildable from entities at any time, not because replacement
// is convenient.
fn authorize(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Update { table_name, .. } | AuthAction::Delete { table_name } => {
            if table_name == "current_parcels" {
                Authorization::Allow
            } else if is_record_table(table_name) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }
        _ => Authorization::Allow,
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::migrate::migrate;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn authorizer_denies_update_on_entities_and_allows_current_parcels() {
        let conn = open();
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (1, 'src-test', 'success', '2026-08-10')",
            [],
        )
        .expect("run");
        conn.execute(
            "INSERT INTO entities (id, load_run_id, county_fips, muni_code, taxkey)
             VALUES (1, 1, '55133', 'WAKC', 'T1')",
            [],
        )
        .expect("entity");
        conn.execute(
            "INSERT INTO current_parcels (
                 county_fips, muni_code, taxkey, id, load_run_id, materialised_at
             ) VALUES ('55133', 'WAKC', 'T1', 1, 1, '2026-08-10')",
            [],
        )
        .expect("current");
        let denied = conn.execute("UPDATE entities SET owner_name = 'x' WHERE id = 1", []);
        assert!(denied.is_err(), "expected deny on entities, got {denied:?}");
        conn.execute(
            "UPDATE current_parcels SET owner_name = 'x' WHERE taxkey = 'T1'",
            [],
        )
        .expect("current_parcels update allowed");
        let name: String = conn
            .query_row(
                "SELECT owner_name FROM current_parcels WHERE taxkey = 'T1'",
                [],
                |row| row.get(0),
            )
            .expect("read");
        assert_eq!(name, "x");
    }
}
