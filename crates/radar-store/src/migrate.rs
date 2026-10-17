use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};

use crate::error::StoreError;

/// Per-connection wait when SQLite returns SQLITE_BUSY. Brief on purpose:
/// WAL is what lets a reader proceed while another connection writes.
pub const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

pub fn configure_connection(conn: &Connection) -> Result<(), StoreError> {
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    enable_wal(conn)
}

fn enable_wal(conn: &Connection) -> Result<(), StoreError> {
    let path = conn.path().unwrap_or("");
    if path.is_empty() || path == ":memory:" {
        return Ok(());
    }
    match conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get::<_, String>(0)) {
        Ok(_) => Ok(()),
        Err(err) => {
            let text = err.to_string();
            if text.contains("readonly") || text.contains("read-only") {
                Ok(())
            } else {
                Err(StoreError::from(err))
            }
        }
    }
}

pub struct Migration {
    pub filename: &'static str,
    pub sql: &'static str,
}

pub fn bundled_migrations() -> Vec<Migration> {
    vec![
        Migration {
            filename: "001_initial.sql",
            sql: include_str!("../migrations/001_initial.sql"),
        },
        Migration {
            filename: "002_muni_lookup_county_fips.sql",
            sql: include_str!("../migrations/002_muni_lookup_county_fips.sql"),
        },
        Migration {
            filename: "003_term_glossary.sql",
            sql: include_str!("../migrations/003_term_glossary.sql"),
        },
        Migration {
            filename: "004_composite_parcel_key.sql",
            sql: include_str!("../migrations/004_composite_parcel_key.sql"),
        },
        Migration {
            filename: "005_signal_catalog.sql",
            sql: include_str!("../migrations/005_signal_catalog.sql"),
        },
        Migration {
            filename: "006_plan_commission_source_and_detection_terms.sql",
            sql: include_str!("../migrations/006_plan_commission_source_and_detection_terms.sql"),
        },
        Migration {
            filename: "007_detections.sql",
            sql: include_str!("../migrations/007_detections.sql"),
        },
        Migration {
            filename: "008_improvement_zero_to_value_cadence.sql",
            sql: include_str!("../migrations/008_improvement_zero_to_value_cadence.sql"),
        },
        Migration {
            filename: "009_current_entities_window.sql",
            sql: include_str!("../migrations/009_current_entities_window.sql"),
        },
        Migration {
            filename: "010_read_indexes.sql",
            sql: include_str!("../migrations/010_read_indexes.sql"),
        },
        Migration {
            filename: "011_current_parcels.sql",
            sql: include_str!("../migrations/011_current_parcels.sql"),
        },
        Migration {
            filename: "012_series.sql",
            sql: include_str!("../migrations/012_series.sql"),
        },
        Migration {
            filename: "013_assignments.sql",
            sql: include_str!("../migrations/013_assignments.sql"),
        },
        Migration {
            filename: "014_placeholder_glossary.sql",
            sql: include_str!("../migrations/014_placeholder_glossary.sql"),
        },
        Migration {
            filename: "015_nonprofit_tribal_glossary.sql",
            sql: include_str!("../migrations/015_nonprofit_tribal_glossary.sql"),
        },
        Migration {
            filename: "016_outcomes.sql",
            sql: include_str!("../migrations/016_outcomes.sql"),
        },
        Migration {
            filename: "017_liquor_licences.sql",
            sql: include_str!("../migrations/017_liquor_licences.sql"),
        },
        Migration {
            filename: "018_current_parcels_owner_group_index.sql",
            sql: include_str!("../migrations/018_current_parcels_owner_group_index.sql"),
        },
        Migration {
            filename: "019_lira_applications.sql",
            sql: include_str!("../migrations/019_lira_applications.sql"),
        },
        Migration {
            filename: "020_lira_non_renewal_exclude_null_type.sql",
            sql: include_str!("../migrations/020_lira_non_renewal_exclude_null_type.sql"),
        },
        Migration {
            filename: "021_lira_pdf_extractions.sql",
            sql: include_str!("../migrations/021_lira_pdf_extractions.sql"),
        },
        Migration {
            filename: "022_accela_occupancy.sql",
            sql: include_str!("../migrations/022_accela_occupancy.sql"),
        },
        Migration {
            filename: "023_accela_occupancy_loads.sql",
            sql: include_str!("../migrations/023_accela_occupancy_loads.sql"),
        },
        Migration {
            filename: "024_vertical_catalog_descriptions.sql",
            sql: include_str!("../migrations/024_vertical_catalog_descriptions.sql"),
        },
        Migration {
            filename: "025_vertical_catalog_elicited.sql",
            sql: include_str!("../migrations/025_vertical_catalog_elicited.sql"),
        },
        Migration {
            filename: "026_vertical_catalog_county_counts.sql",
            sql: include_str!("../migrations/026_vertical_catalog_county_counts.sql"),
        },
    ]
}

pub fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    configure_connection(conn)?;
    apply_migrations(conn, &bundled_migrations())?;
    crate::authorizer::install_write_authorizer(conn);
    Ok(())
}

pub fn apply_migrations(conn: &mut Connection, migrations: &[Migration]) -> Result<(), StoreError> {
    configure_connection(conn)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            filename TEXT PRIMARY KEY,
            checksum TEXT NOT NULL,
            applied_at TEXT NOT NULL
        )",
        [],
    )?;

    for migration in migrations {
        let checksum = checksum_sql(migration.sql);
        let existing: Option<String> = conn
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE filename = ?1",
                [migration.filename],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(applied) if applied == checksum => continue,
            Some(_) => {
                return Err(StoreError::ChecksumMismatch {
                    filename: migration.filename.to_string(),
                });
            }
            None => {
                let tx = conn.transaction()?;
                tx.execute_batch(migration.sql)?;
                tx.execute(
                    "INSERT INTO schema_migrations (filename, checksum, applied_at)
                     VALUES (?1, ?2, datetime('now'))",
                    [migration.filename, checksum.as_str()],
                )?;
                tx.commit()?;
            }
        }
    }
    Ok(())
}

fn checksum_sql(sql: &str) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for byte in sql.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{configure_connection, migrate};

    #[test]
    fn file_backed_store_uses_wal_and_busy_timeout() {
        let path = std::env::temp_dir().join(format!(
            "radar-wal-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let mut conn = Connection::open(&path).expect("open temp");
        migrate(&mut conn).expect("migrate");
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(mode.to_ascii_lowercase(), "wal");
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout");
        assert_eq!(timeout, 5_000);
        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let stem = path.display().to_string();
        let _ = std::fs::remove_file(format!("{stem}-wal"));
        let _ = std::fs::remove_file(format!("{stem}-shm"));
    }

    #[test]
    fn configure_is_safe_on_memory() {
        let conn = Connection::open_in_memory().expect("memory");
        configure_connection(&conn).expect("configure");
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout");
        assert_eq!(timeout, 5_000);
    }
}
