use rusqlite::{Connection, OptionalExtension};

use crate::error::StoreError;

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
    ]
}

pub fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    apply_migrations(conn, &bundled_migrations())?;
    crate::authorizer::install_write_authorizer(conn);
    Ok(())
}

pub fn apply_migrations(conn: &mut Connection, migrations: &[Migration]) -> Result<(), StoreError> {
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
