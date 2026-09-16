use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::error::StoreError;

const CURRENT_COLUMNS: &str = "county_fips, muni_code, taxkey, id, load_run_id,
    owner_name, site_address, mail_address_1, mail_address_2,
    assessed_value, improvement_value, land_value, owner_type,
    mail_matches_site, prop_class, value_source, value_as_of,
    source_layer_url, fetched_at, rec_source";

const SELECT_FROM_ENTITIES: &str = "county_fips, muni_code, taxkey, id, load_run_id,
    owner_name, site_address, mail_address_1, mail_address_2,
    assessed_value, improvement_value, land_value, owner_type,
    mail_matches_site, prop_class, value_source, value_as_of,
    source_layer_url, fetched_at, rec_source";

pub fn mirror_entity(conn: &Connection, entity_id: i64) -> Result<(), StoreError> {
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO current_parcels (
                 {CURRENT_COLUMNS}, materialised_at
             )
             SELECT {SELECT_FROM_ENTITIES}, datetime('now')
             FROM entities
             WHERE id = ?1
               AND county_fips IS NOT NULL
               AND muni_code IS NOT NULL"
        ),
        [entity_id],
    )?;
    Ok(())
}

pub fn mirror_latest_entity(conn: &Connection) -> Result<(), StoreError> {
    mirror_entity(conn, conn.last_insert_rowid())
}

pub fn replace_run_keys(
    tx: &Transaction<'_>,
    load_run_id: i64,
    county_fips: &str,
) -> Result<i64, StoreError> {
    let other_before: i64 = tx.query_row(
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips != ?1",
        [county_fips],
        |row| row.get(0),
    )?;
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO current_parcels (
                 {CURRENT_COLUMNS}, materialised_at
             )
             SELECT {SELECT_FROM_ENTITIES}, datetime('now')
             FROM entities
             WHERE load_run_id = ?1
               AND county_fips = ?2
               AND muni_code IS NOT NULL"
        ),
        params![load_run_id, county_fips],
    )?;
    let written = tx.changes() as i64;
    let other_after: i64 = tx.query_row(
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips != ?1",
        [county_fips],
        |row| row.get(0),
    )?;
    println!(
        "current_parcels county_fips={county_fips} rows_written={written} other_county_before={other_before} other_county_after={other_after}"
    );
    if other_after != other_before {
        return Err(StoreError::ParcelAssertion {
            code: "L".to_string(),
            expected: format!("other_county_rows={other_before}"),
            actual: format!("other_county_rows={other_after}"),
        });
    }
    let leaked: i64 = tx.query_row(
        "SELECT COUNT(*) FROM current_parcels
         WHERE load_run_id = ?1 AND county_fips != ?2",
        params![load_run_id, county_fips],
        |row| row.get(0),
    )?;
    if leaked != 0 {
        return Err(StoreError::ParcelAssertion {
            code: "L".to_string(),
            expected: "written keys stay inside fetched county".to_string(),
            actual: format!("leaked={leaked}"),
        });
    }
    Ok(written)
}

pub fn assert_agrees_with_view(
    tx: &Transaction<'_>,
    county_fips: &str,
) -> Result<(), StoreError> {
    let expected: i64 = tx.query_row(
        "SELECT COUNT(*) FROM current_entities WHERE county_fips = ?1",
        [county_fips],
        |row| row.get(0),
    )?;
    let actual: i64 = tx.query_row(
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1",
        [county_fips],
        |row| row.get(0),
    )?;
    println!("L expected={expected} actual={actual}");
    if expected != actual {
        return Err(StoreError::ParcelAssertion {
            code: "L".to_string(),
            expected: format!("current_parcels={expected} matching current_entities"),
            actual: format!("current_parcels={actual}"),
        });
    }
    Ok(())
}

pub fn rebuild_current_parcels(
    conn: &Connection,
    county_fips: Option<&str>,
) -> Result<i64, StoreError> {
    match county_fips {
        Some(county) => {
            conn.execute(
                "DELETE FROM current_parcels WHERE county_fips = ?1",
                [county],
            )?;
            conn.execute(
                &format!(
                    "INSERT INTO current_parcels (
                         {CURRENT_COLUMNS}, materialised_at
                     )
                     SELECT {CURRENT_COLUMNS}, datetime('now')
                     FROM current_entities
                     WHERE county_fips = ?1
                       AND muni_code IS NOT NULL"
                ),
                [county],
            )?;
        }
        None => {
            conn.execute("DELETE FROM current_parcels", [])?;
            conn.execute(
                &format!(
                    "INSERT INTO current_parcels (
                         {CURRENT_COLUMNS}, materialised_at
                     )
                     SELECT {CURRENT_COLUMNS}, datetime('now')
                     FROM current_entities
                     WHERE county_fips IS NOT NULL
                       AND muni_code IS NOT NULL"
                ),
                [],
            )?;
        }
    }
    Ok(conn.changes() as i64)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountyAgreement {
    pub county_fips: String,
    pub view_count: i64,
    pub table_count: i64,
    pub only_in_view: Vec<String>,
    pub only_in_table: Vec<String>,
    pub differing: Vec<String>,
}

impl CountyAgreement {
    pub fn agrees(&self) -> bool {
        self.view_count == self.table_count
            && self.only_in_view.is_empty()
            && self.only_in_table.is_empty()
            && self.differing.is_empty()
    }
}

pub fn check_current_parcels_agreement(
    conn: &Connection,
) -> Result<Vec<CountyAgreement>, StoreError> {
    let mut counties: Vec<String> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT county_fips FROM current_entities WHERE county_fips IS NOT NULL
             UNION
             SELECT county_fips FROM current_parcels WHERE county_fips IS NOT NULL
             ORDER BY 1",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        for row in rows {
            counties.push(row?);
        }
    }
    let mut reports = Vec::new();
    for county in counties {
        reports.push(agree_one_county(conn, &county)?);
    }
    Ok(reports)
}

fn agree_one_county(conn: &Connection, county: &str) -> Result<CountyAgreement, StoreError> {
    let view_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM current_entities WHERE county_fips = ?1",
        [county],
        |row| row.get(0),
    )?;
    let table_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1",
        [county],
        |row| row.get(0),
    )?;
    let mut only_in_view = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT e.county_fips, e.muni_code, e.taxkey
             FROM current_entities e
             WHERE e.county_fips = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM current_parcels p
                   WHERE p.county_fips = e.county_fips
                     AND p.muni_code IS e.muni_code
                     AND p.taxkey = e.taxkey
               )
             LIMIT 5",
        )?;
        let rows = stmt.query_map([county], |row| {
            Ok(format!(
                "{}|{}|{}",
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, String>(2)?
            ))
        })?;
        for row in rows {
            only_in_view.push(row?);
        }
    }
    let mut only_in_table = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT p.county_fips, p.muni_code, p.taxkey
             FROM current_parcels p
             WHERE p.county_fips = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM current_entities e
                   WHERE e.county_fips = p.county_fips
                     AND e.muni_code IS p.muni_code
                     AND e.taxkey = p.taxkey
               )
             LIMIT 5",
        )?;
        let rows = stmt.query_map([county], |row| {
            Ok(format!(
                "{}|{}|{}",
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, String>(2)?
            ))
        })?;
        for row in rows {
            only_in_table.push(row?);
        }
    }
    let mut differing = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT e.county_fips, e.muni_code, e.taxkey,
                    e.id, p.id,
                    e.load_run_id, p.load_run_id,
                    e.owner_name, p.owner_name,
                    e.site_address, p.site_address,
                    e.mail_address_1, p.mail_address_1,
                    e.mail_address_2, p.mail_address_2,
                    e.assessed_value, p.assessed_value,
                    e.improvement_value, p.improvement_value,
                    e.land_value, p.land_value,
                    e.owner_type, p.owner_type,
                    e.mail_matches_site, p.mail_matches_site,
                    e.prop_class, p.prop_class,
                    e.value_source, p.value_source,
                    e.value_as_of, p.value_as_of,
                    e.source_layer_url, p.source_layer_url,
                    e.fetched_at, p.fetched_at,
                    e.rec_source, p.rec_source
             FROM current_entities e
             INNER JOIN current_parcels p
               ON p.county_fips = e.county_fips
              AND p.muni_code IS e.muni_code
              AND p.taxkey = e.taxkey
             WHERE e.county_fips = ?1
             LIMIT 200",
        )?;
        let rows = stmt.query_map([county], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                collect_diffs(row)?,
            ))
        })?;
        for row in rows {
            let (c, m, t, diffs) = row?;
            if !diffs.is_empty() {
                differing.push(format!(
                    "{}|{}|{} columns={}",
                    c,
                    m.unwrap_or_default(),
                    t,
                    diffs.join(",")
                ));
            }
            if differing.len() >= 5 {
                break;
            }
        }
    }
    Ok(CountyAgreement {
        county_fips: county.to_string(),
        view_count,
        table_count,
        only_in_view,
        only_in_table,
        differing,
    })
}

fn collect_diffs(row: &rusqlite::Row<'_>) -> rusqlite::Result<Vec<String>> {
    let names = [
        "id",
        "load_run_id",
        "owner_name",
        "site_address",
        "mail_address_1",
        "mail_address_2",
        "assessed_value",
        "improvement_value",
        "land_value",
        "owner_type",
        "mail_matches_site",
        "prop_class",
        "value_source",
        "value_as_of",
        "source_layer_url",
        "fetched_at",
        "rec_source",
    ];
    let mut diffs = Vec::new();
    let mut idx = 3;
    for name in names {
        let left: Option<String> = value_as_text(row, idx)?;
        let right: Option<String> = value_as_text(row, idx + 1)?;
        idx += 2;
        if left != right {
            diffs.push(name.to_string());
        }
    }
    Ok(diffs)
}

fn value_as_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<String>> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Null => Ok(None),
        rusqlite::types::ValueRef::Integer(v) => Ok(Some(v.to_string())),
        rusqlite::types::ValueRef::Real(v) => Ok(Some(v.to_string())),
        rusqlite::types::ValueRef::Text(v) => Ok(Some(
            std::str::from_utf8(v)
                .map_err(|err| rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(err)))?
                .to_string(),
        )),
        rusqlite::types::ValueRef::Blob(_) => Ok(Some("<blob>".to_string())),
    }
}

pub fn print_agreement(reports: &[CountyAgreement]) {
    for report in reports {
        let status = if report.agrees() { "AGREE" } else { "DISAGREE" };
        println!(
            "county_fips={} view_count={} table_count={} {}",
            report.county_fips, report.view_count, report.table_count, status
        );
        for key in &report.only_in_view {
            println!("  only_in_current_entities {key}");
        }
        for key in &report.only_in_table {
            println!("  only_in_current_parcels {key}");
        }
        for row in &report.differing {
            println!("  differ {row}");
        }
    }
}

pub fn sample_value_as_of(
    conn: &Connection,
    county_fips: &str,
    muni_code: &str,
    taxkey: &str,
) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT value_as_of FROM current_parcels
             WHERE county_fips = ?1 AND muni_code = ?2 AND taxkey = ?3",
            params![county_fips, muni_code, taxkey],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::{
        check_current_parcels_agreement, rebuild_current_parcels, replace_run_keys,
        sample_value_as_of,
    };
    use crate::migrate::migrate;
    use rusqlite::{params, Connection};

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    fn seed_run(conn: &Connection, id: i64, finished: &str) {
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (?1, 'src-waukesha-parcels', 'success', ?2)",
            params![id, finished],
        )
        .unwrap();
    }

    fn seed_entity(
        conn: &Connection,
        id: i64,
        run: i64,
        county: &str,
        muni: &str,
        taxkey: &str,
        value_as_of: &str,
        name: &str,
    ) {
        conn.execute(
            "INSERT INTO entities (
                 id, load_run_id, county_fips, muni_code, taxkey, owner_name, value_as_of
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, run, county, muni, taxkey, name, value_as_of],
        )
        .unwrap();
    }

    #[test]
    fn rebuild_from_empty_matches_the_view() {
        let conn = open();
        seed_run(&conn, 1, "2026-01-01");
        seed_run(&conn, 2, "2026-02-01");
        seed_entity(&conn, 1, 1, "55133", "WAKC", "T1", "2026-01-01", "OLD");
        seed_entity(&conn, 2, 2, "55133", "WAKC", "T1", "2026-02-01", "NEW");
        seed_entity(&conn, 3, 1, "55133", "WAKC", "T2", "2026-01-01", "KEEP");
        seed_entity(&conn, 4, 2, "55079", "53000", "M1", "2026-02-01", "MKE");
        rebuild_current_parcels(&conn, None).unwrap();
        let reports = check_current_parcels_agreement(&conn).unwrap();
        assert!(reports.iter().all(|r| r.agrees()), "{reports:?}");
        let name: String = conn
            .query_row(
                "SELECT owner_name FROM current_parcels WHERE taxkey = 'T1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "NEW");
        let kept: String = conn
            .query_row(
                "SELECT value_as_of FROM current_parcels WHERE taxkey = 'T2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, "2026-01-01");
    }

    #[test]
    fn agreement_check_detects_injected_discrepancy() {
        let conn = open();
        seed_run(&conn, 1, "2026-01-01");
        seed_entity(&conn, 1, 1, "55133", "WAKC", "T1", "2026-01-01", "A");
        rebuild_current_parcels(&conn, None).unwrap();
        conn.execute(
            "UPDATE current_parcels SET owner_name = 'INJECTED' WHERE taxkey = 'T1'",
            [],
        )
        .unwrap();
        let reports = check_current_parcels_agreement(&conn).unwrap();
        let waukesha = reports.iter().find(|r| r.county_fips == "55133").unwrap();
        assert!(!waukesha.agrees());
        assert!(waukesha
            .differing
            .iter()
            .any(|line| line.contains("owner_name")));
    }

    #[test]
    fn replace_run_keys_leaves_other_county_untouched() {
        let mut conn = open();
        seed_run(&conn, 1, "2026-01-01");
        seed_run(&conn, 2, "2026-02-01");
        seed_entity(&conn, 1, 1, "55133", "WAKC", "T1", "2026-01-01", "W");
        seed_entity(&conn, 2, 1, "55079", "53000", "M1", "2026-01-01", "M");
        rebuild_current_parcels(&conn, None).unwrap();
        seed_entity(&conn, 3, 2, "55133", "WAKC", "T1", "2026-02-01", "W2");
        let tx = conn.transaction().unwrap();
        replace_run_keys(&tx, 2, "55133").unwrap();
        tx.commit().unwrap();
        let w: String = conn
            .query_row(
                "SELECT owner_name FROM current_parcels WHERE taxkey = 'T1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(w, "W2");
        let m: String = conn
            .query_row(
                "SELECT owner_name FROM current_parcels WHERE taxkey = 'M1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(m, "M");
        assert_eq!(
            sample_value_as_of(&conn, "55079", "53000", "M1")
                .unwrap()
                .as_deref(),
            Some("2026-01-01")
        );
    }
}
