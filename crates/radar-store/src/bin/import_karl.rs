use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use radar_store::migrate;
use rusqlite::{params, Connection, OpenFlags};

struct Check {
    code: &'static str,
    expected: String,
    actual: String,
    passed: bool,
    extra: Vec<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: import_karl <source-db> <destination-db>");
        return ExitCode::from(2);
    }
    let source_path = PathBuf::from(&args[1]);
    let dest_path = PathBuf::from(&args[2]);

    if source_path == dest_path {
        eprintln!("source and destination paths must differ");
        return ExitCode::from(2);
    }

    let source = match open_source_ro(&source_path) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("failed to open source read-only: {err}");
            return ExitCode::from(1);
        }
    };

    if let Some(parent) = dest_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                eprintln!("failed to create destination directory: {err}");
                return ExitCode::from(1);
            }
        }
    }

    let mut dest = match Connection::open(&dest_path) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("failed to open destination: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = migrate(&mut dest) {
        eprintln!("failed to migrate destination: {err}");
        return ExitCode::from(1);
    }

    let checks = match run_source_assertions(&source) {
        Ok(checks) => checks,
        Err(err) => {
            eprintln!("failed to run source assertions: {err}");
            return ExitCode::from(1);
        }
    };
    print_checks(&checks);
    let failed: Vec<&Check> = checks.iter().filter(|check| !check.passed).collect();
    if !failed.is_empty() {
        let names = failed
            .iter()
            .map(|check| check.code)
            .collect::<Vec<_>>()
            .join(", ");
        let error_text = format!("failed assertion {names}");
        if let Err(err) = insert_failed_run(&dest, &error_text) {
            eprintln!("failed to record failed load_run: {err}");
        }
        if let Err(err) = print_load_run(&dest) {
            eprintln!("failed to print load_run: {err}");
        }
        return ExitCode::from(1);
    }

    match import(&source, &mut dest) {
        Ok(()) => {}
        Err(err) => {
            eprintln!("import failed: {err}");
            if let Err(record_err) = insert_failed_run(&dest, &format!("import failed: {err}")) {
                eprintln!("failed to record failed load_run: {record_err}");
            }
            let _ = print_load_run(&dest);
            return ExitCode::from(1);
        }
    }

    if let Err(err) = print_load_run(&dest) {
        eprintln!("failed to print load_run: {err}");
        return ExitCode::from(1);
    }
    if let Err(err) = print_profile(&dest) {
        eprintln!("failed to print profile: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn open_source_ro(path: &Path) -> Result<Connection, rusqlite::Error> {
    if !path.exists() {
        return Err(rusqlite::Error::InvalidPath(path.to_path_buf()));
    }
    let unix = path.to_string_lossy().replace('\\', "/");
    let uri = format!("file:{unix}?mode=ro");
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .and_then(|conn| {
        let _ = radar_store::configure_connection(&conn);
        Ok(conn)
    })
}

fn run_source_assertions(source: &Connection) -> Result<Vec<Check>, rusqlite::Error> {
    let mut checks = Vec::new();

    let entity_count: i64 = source.query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))?;
    checks.push(Check {
        code: "A",
        expected: "142752".to_string(),
        actual: entity_count.to_string(),
        passed: entity_count == 142752,
        extra: Vec::new(),
    });

    let distinct_taxkey: i64 =
        source.query_row("SELECT COUNT(DISTINCT taxkey) FROM entities", [], |row| row.get(0))?;
    let mut b_extra = Vec::new();
    if distinct_taxkey != entity_count {
        let mut stmt = source.prepare(
            "SELECT taxkey, COUNT(*) AS c
             FROM entities
             GROUP BY taxkey
             HAVING c > 1
             ORDER BY c DESC, taxkey
             LIMIT 10",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(format!(
                "taxkey={} count={}",
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?
            ))
        })?;
        for row in rows {
            b_extra.push(row?);
        }
    }
    checks.push(Check {
        code: "B",
        expected: entity_count.to_string(),
        actual: distinct_taxkey.to_string(),
        passed: distinct_taxkey == entity_count,
        extra: b_extra,
    });

    let owner_null: i64 = source.query_row(
        "SELECT COUNT(*) FROM entities WHERE owner_name IS NULL",
        [],
        |row| row.get(0),
    )?;
    let owner_blank: i64 = source.query_row(
        "SELECT COUNT(*) FROM entities WHERE owner_name = ''",
        [],
        |row| row.get(0),
    )?;
    checks.push(Check {
        code: "C",
        expected: "null=0 blank=0".to_string(),
        actual: format!("null={owner_null} blank={owner_blank}"),
        passed: owner_null == 0 && owner_blank == 0,
        extra: Vec::new(),
    });

    let mut type_counts = std::collections::BTreeMap::new();
    let mut type_stmt = source.prepare("SELECT owner_type, COUNT(*) FROM entities GROUP BY owner_type")?;
    let type_rows = type_stmt.query_map([], |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in type_rows {
        let (owner_type, count) = row?;
        type_counts.insert(owner_type.unwrap_or_else(|| "<null>".to_string()), count);
    }
    let expected_types = [
        ("individual", 119682_i64),
        ("trust", 18457),
        ("entity", 4611),
        ("government", 2),
    ];
    let mut d_ok = type_counts.len() == expected_types.len();
    for (name, count) in expected_types {
        if type_counts.get(name) != Some(&count) {
            d_ok = false;
        }
    }
    let actual_types = type_counts
        .iter()
        .map(|(name, count)| format!("{name}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    checks.push(Check {
        code: "D",
        expected: "individual=119682 trust=18457 entity=4611 government=2".to_string(),
        actual: actual_types,
        passed: d_ok,
        extra: Vec::new(),
    });

    let imp_null: i64 = source.query_row(
        "SELECT COUNT(*) FROM entities WHERE improvement_value IS NULL",
        [],
        |row| row.get(0),
    )?;
    let imp_zero: i64 = source.query_row(
        "SELECT COUNT(*) FROM entities WHERE improvement_value = 0",
        [],
        |row| row.get(0),
    )?;
    checks.push(Check {
        code: "E",
        expected: "null=0 zero=4478".to_string(),
        actual: format!("null={imp_null} zero={imp_zero}"),
        passed: imp_null == 0 && imp_zero == 4478,
        extra: Vec::new(),
    });

    let prop_distinct: i64 = source.query_row(
        "SELECT COUNT(DISTINCT prop_class) FROM entities",
        [],
        |row| row.get(0),
    )?;
    checks.push(Check {
        code: "F",
        expected: "1".to_string(),
        actual: prop_distinct.to_string(),
        passed: prop_distinct == 1,
        extra: Vec::new(),
    });

    let mut as_of_counts = std::collections::BTreeMap::new();
    let mut as_of_stmt =
        source.prepare("SELECT value_as_of, COUNT(*) FROM entities GROUP BY value_as_of")?;
    let as_of_rows = as_of_stmt.query_map([], |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in as_of_rows {
        let (value, count) = row?;
        as_of_counts.insert(value.unwrap_or_else(|| "<null>".to_string()), count);
    }
    let g_ok = as_of_counts.len() == 2
        && as_of_counts.get("2025-12-31") == Some(&83610)
        && as_of_counts.get("2026-08-10") == Some(&59142);
    let actual_as_of = as_of_counts
        .iter()
        .map(|(name, count)| format!("{name}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    checks.push(Check {
        code: "G",
        expected: "2025-12-31=83610 2026-08-10=59142".to_string(),
        actual: actual_as_of,
        passed: g_ok,
        extra: Vec::new(),
    });

    let muni_count: i64 = source.query_row("SELECT COUNT(*) FROM waukesha_muni_lookup", [], |row| {
        row.get(0)
    })?;
    checks.push(Check {
        code: "H",
        expected: "38".to_string(),
        actual: muni_count.to_string(),
        passed: muni_count == 38,
        extra: Vec::new(),
    });

    let entity_muni: i64 = source.query_row(
        "SELECT COUNT(DISTINCT muni_code) FROM entities WHERE muni_code IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let lookup_muni: i64 = source.query_row(
        "SELECT COUNT(DISTINCT muni_code) FROM waukesha_muni_lookup",
        [],
        |row| row.get(0),
    )?;
    let matched: i64 = source.query_row(
        "SELECT COUNT(DISTINCT e.muni_code)
         FROM entities e
         INNER JOIN waukesha_muni_lookup m ON m.muni_code = e.muni_code",
        [],
        |row| row.get(0),
    )?;
    println!(
        "I muni_code join match count: {matched} of {entity_muni} distinct entity codes; {matched} of {lookup_muni} distinct lookup codes ({matched}/{entity_muni} and {matched}/{lookup_muni})"
    );
    let mut unmatched = Vec::new();
    let mut unmatched_stmt = source.prepare(
        "SELECT DISTINCT e.muni_code
         FROM entities e
         WHERE e.muni_code IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM waukesha_muni_lookup m WHERE m.muni_code = e.muni_code
           )
         ORDER BY e.muni_code",
    )?;
    let unmatched_rows = unmatched_stmt.query_map([], |row| row.get::<_, String>(0))?;
    for row in unmatched_rows {
        unmatched.push(format!("unmatched entity muni_code={}", row?));
    }
    checks.push(Check {
        code: "I",
        expected: format!("all {entity_muni} entity muni_codes resolve"),
        actual: format!("matched={matched} unmatched={}", unmatched.len()),
        passed: unmatched.is_empty() && matched == entity_muni,
        extra: unmatched,
    });

    Ok(checks)
}

fn print_checks(checks: &[Check]) {
    for check in checks {
        let result = if check.passed { "PASS" } else { "FAIL" };
        println!(
            "{} expected={} actual={} {}",
            check.code, check.expected, check.actual, result
        );
        for line in &check.extra {
            println!("  {}", line);
        }
    }
}

fn unique_or_none(values: &[String]) -> Option<String> {
    let first = values.first()?;
    if values.iter().all(|value| value == first) {
        Some(first.clone())
    } else {
        None
    }
}

fn source_run_metadata(
    source: &Connection,
) -> Result<(Option<String>, Option<String>, Option<String>, Option<String>), rusqlite::Error> {
    let mut stmt = source.prepare(
        "SELECT layer_url, where_clause, value_source, value_as_of
         FROM entity_load_runs",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut layer_urls = Vec::new();
    let mut where_clauses = Vec::new();
    let mut value_sources = Vec::new();
    let mut values_as_of = Vec::new();
    for row in rows {
        let (layer_url, where_clause, value_source, value_as_of) = row?;
        println!(
            "source entity_load_runs layer_url={:?} where_clause={:?} value_source={:?} value_as_of={:?}",
            layer_url, where_clause, value_source, value_as_of
        );
        if let Some(value) = layer_url {
            layer_urls.push(value);
        }
        if let Some(value) = where_clause {
            where_clauses.push(value);
        }
        if let Some(value) = value_source {
            value_sources.push(value);
        }
        if let Some(value) = value_as_of {
            values_as_of.push(value);
        }
    }
    Ok((
        unique_or_none(&layer_urls),
        unique_or_none(&where_clauses),
        unique_or_none(&value_sources),
        unique_or_none(&values_as_of),
    ))
}

fn insert_failed_run(dest: &Connection, error_text: &str) -> Result<(), rusqlite::Error> {
    dest.execute(
        "INSERT INTO load_runs (source_id, status, started_at, finished_at, error_text)
         VALUES ('src-waukesha-parcels', 'failed', datetime('now'), datetime('now'), ?1)",
        [error_text],
    )?;
    Ok(())
}

fn import(source: &Connection, dest: &mut Connection) -> Result<(), String> {
    let (layer_url, where_clause, value_source, value_as_of) =
        source_run_metadata(source).map_err(|err| err.to_string())?;
    if layer_url.is_none() {
        println!("destination load_run.layer_url left unset: source has no single layer_url");
    }
    if where_clause.is_none() {
        println!("destination load_run.where_clause left unset: source has no single where_clause");
    }
    if value_source.is_none() {
        println!("destination load_run.value_source left unset: source has no single value_source");
    }
    if value_as_of.is_none() {
        println!("destination load_run.value_as_of left unset: source has no single value_as_of");
    }

    let tx = dest.transaction().map_err(|err| err.to_string())?;
    tx.execute(
        "INSERT INTO load_runs (
             source_id, layer_url, where_clause, value_source, value_as_of,
             started_at, finished_at, status, records_requested, records_written,
             records_skipped, error_text
         ) VALUES (
             'src-waukesha-parcels', ?1, ?2, ?3, ?4,
             datetime('now'), datetime('now'), 'success', 142752, 142752,
             0, NULL
         )",
        params![layer_url, where_clause, value_source, value_as_of],
    )
    .map_err(|err| err.to_string())?;
    let load_run_id = tx.last_insert_rowid();

    {
        let mut insert_muni = tx
            .prepare(
                "INSERT INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
                 VALUES ('55133', ?1, ?2, ?3, ?4)",
            )
            .map_err(|err| err.to_string())?;
        let mut source_muni = source
            .prepare("SELECT muni_code, muni_name, source FROM waukesha_muni_lookup")
            .map_err(|err| err.to_string())?;
        let muni_rows = source_muni
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|err| err.to_string())?;
        for row in muni_rows {
            let (muni_code, muni_name, muni_source) = row.map_err(|err| err.to_string())?;
            insert_muni
                .execute(params![muni_code, muni_name, muni_source, load_run_id])
                .map_err(|err| err.to_string())?;
        }
    }

    {
        let mut insert_entity = tx
            .prepare(
                "INSERT INTO entities (
                     load_run_id, county_fips, taxkey, muni_code, owner_name,
                     site_address, mail_address_1, mail_address_2, assessed_value,
                     improvement_value, land_value, owner_type, mail_matches_site,
                     prop_class, value_source, value_as_of, source_layer_url, fetched_at
                 ) VALUES (
                     ?1, ?2, ?3, ?4, ?5,
                     ?6, ?7, ?8, ?9,
                     ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18
                 )",
            )
            .map_err(|err| err.to_string())?;
        let mut source_entities = source
            .prepare(
                "SELECT county_fips, taxkey, muni_code, owner_name,
                        site_address, mail_address_1, mail_address_2, assessed_value,
                        improvement_value, land_value, owner_type, mail_matches_site,
                        prop_class, value_source, value_as_of, source_layer_url, fetched_at
                 FROM entities",
            )
            .map_err(|err| err.to_string())?;
        let entity_rows = source_entities
            .query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<f64>>(7)?,
                    row.get::<_, Option<f64>>(8)?,
                    row.get::<_, Option<f64>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<String>>(16)?,
                ))
            })
            .map_err(|err| err.to_string())?;
        for row in entity_rows {
            let (
                county_fips,
                taxkey,
                muni_code,
                owner_name,
                site_address,
                mail_address_1,
                mail_address_2,
                assessed_value,
                improvement_value,
                land_value,
                owner_type,
                mail_matches_site,
                prop_class,
                value_source,
                value_as_of,
                source_layer_url,
                fetched_at,
            ) = row.map_err(|err| err.to_string())?;
            insert_entity
                .execute(params![
                    load_run_id,
                    county_fips,
                    taxkey,
                    muni_code,
                    owner_name,
                    site_address,
                    mail_address_1,
                    mail_address_2,
                    assessed_value,
                    improvement_value,
                    land_value,
                    owner_type,
                    mail_matches_site,
                    prop_class,
                    value_source,
                    value_as_of,
                    source_layer_url,
                    fetched_at,
                ])
                .map_err(|err| err.to_string())?;
        }
    }

    let dest_entity_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .map_err(|err| err.to_string())?;
    let dest_distinct_taxkey: i64 = tx
        .query_row("SELECT COUNT(DISTINCT taxkey) FROM entities", [], |row| row.get(0))
        .map_err(|err| err.to_string())?;
    let source_distinct_taxkey: i64 = source
        .query_row("SELECT COUNT(DISTINCT taxkey) FROM entities", [], |row| row.get(0))
        .map_err(|err| err.to_string())?;
    let dest_muni_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM muni_lookup", [], |row| row.get(0))
        .map_err(|err| err.to_string())?;
    let dest_individual: i64 = count_owner_type(&tx, "individual")?;
    let dest_trust: i64 = count_owner_type(&tx, "trust")?;
    let dest_entity: i64 = count_owner_type(&tx, "entity")?;
    let dest_government: i64 = count_owner_type(&tx, "government")?;
    let dest_other_types: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM entities
             WHERE owner_type NOT IN ('individual', 'trust', 'entity', 'government')
                OR owner_type IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|err| err.to_string())?;
    let dest_imp_zero: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE improvement_value = 0",
            [],
            |row| row.get(0),
        )
        .map_err(|err| err.to_string())?;

    println!(
        "post entities count expected=142752 actual={dest_entity_count} {}",
        pass(dest_entity_count == 142752)
    );
    println!(
        "post distinct taxkey expected={source_distinct_taxkey} actual={dest_distinct_taxkey} {}",
        pass(dest_distinct_taxkey == source_distinct_taxkey)
    );
    println!(
        "post muni_lookup count expected=38 actual={dest_muni_count} {}",
        pass(dest_muni_count == 38)
    );
    println!(
        "post owner_type expected=individual=119682 trust=18457 entity=4611 government=2 other=0 actual=individual={dest_individual} trust={dest_trust} entity={dest_entity} government={dest_government} other={dest_other_types} {}",
        pass(
            dest_individual == 119682
                && dest_trust == 18457
                && dest_entity == 4611
                && dest_government == 2
                && dest_other_types == 0
        )
    );
    println!(
        "post improvement_value zero expected=4478 actual={dest_imp_zero} {}",
        pass(dest_imp_zero == 4478)
    );

    let pre_ok = dest_entity_count == 142752
        && dest_distinct_taxkey == source_distinct_taxkey
        && dest_muni_count == 38
        && dest_individual == 119682
        && dest_trust == 18457
        && dest_entity == 4611
        && dest_government == 2
        && dest_other_types == 0
        && dest_imp_zero == 4478;
    if !pre_ok {
        return Err("post-write verification failed before current_entities".to_string());
    }

    tx.execute(
        "CREATE INDEX IF NOT EXISTS entities_taxkey ON entities (taxkey)",
        [],
    )
    .map_err(|err| err.to_string())?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS entities_load_run_id ON entities (load_run_id)",
        [],
    )
    .map_err(|err| err.to_string())?;

    let current_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM current_entities", [], |row| row.get(0))
        .map_err(|err| err.to_string())?;
    println!(
        "post current_entities count expected=142752 actual={current_count} {}",
        pass(current_count == 142752)
    );
    if current_count != 142752 {
        return Err("post-write verification failed: current_entities count".to_string());
    }

    tx.commit().map_err(|err| err.to_string())?;
    Ok(())
}

fn count_owner_type(conn: &Connection, owner_type: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE owner_type = ?1",
        [owner_type],
        |row| row.get(0),
    )
    .map_err(|err| err.to_string())
}

fn pass(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}

fn print_load_run(dest: &Connection) -> Result<(), rusqlite::Error> {
    let id: Option<i64> = dest.query_row("SELECT MAX(id) FROM load_runs", [], |row| row.get(0))?;
    let Some(id) = id else {
        println!("load_run: none");
        return Ok(());
    };
    let mut stmt = dest.prepare(
        "SELECT id, source_id, layer_url, where_clause, value_source, value_as_of,
                started_at, finished_at, status, records_requested, records_written,
                records_skipped, error_text
         FROM load_runs WHERE id = ?1",
    )?;
    stmt.query_row([id], |row| {
        println!("load_run id={}", row.get::<_, i64>(0)?);
        println!("  source_id={}", row.get::<_, String>(1)?);
        println!("  layer_url={:?}", row.get::<_, Option<String>>(2)?);
        println!("  where_clause={:?}", row.get::<_, Option<String>>(3)?);
        println!("  value_source={:?}", row.get::<_, Option<String>>(4)?);
        println!("  value_as_of={:?}", row.get::<_, Option<String>>(5)?);
        println!("  started_at={:?}", row.get::<_, Option<String>>(6)?);
        println!("  finished_at={:?}", row.get::<_, Option<String>>(7)?);
        println!("  status={}", row.get::<_, String>(8)?);
        println!("  records_requested={:?}", row.get::<_, Option<i64>>(9)?);
        println!("  records_written={:?}", row.get::<_, Option<i64>>(10)?);
        println!("  records_skipped={:?}", row.get::<_, Option<i64>>(11)?);
        println!("  error_text={:?}", row.get::<_, Option<String>>(12)?);
        Ok(())
    })
}

fn print_profile(dest: &Connection) -> Result<(), rusqlite::Error> {
    let distinct_owner: i64 =
        dest.query_row("SELECT COUNT(DISTINCT owner_name) FROM entities", [], |row| {
            row.get(0)
        })?;
    println!("profile distinct owner_name={distinct_owner}");

    let entity_type_count: i64 = dest.query_row(
        "SELECT COUNT(*) FROM entities WHERE owner_type = 'entity'",
        [],
        |row| row.get(0),
    )?;
    println!("profile owner_type=entity count={entity_type_count}");

    let entity_zero: i64 = dest.query_row(
        "SELECT COUNT(*) FROM entities
         WHERE owner_type = 'entity' AND improvement_value = 0",
        [],
        |row| row.get(0),
    )?;
    println!("profile improvement_value=0 and owner_type=entity count={entity_zero}");

    println!("profile top 20 owner_name by parcel count where owner_type=entity");
    let mut stmt = dest.prepare(
        "SELECT owner_name, COUNT(*) AS n
         FROM entities
         WHERE owner_type = 'entity'
         GROUP BY owner_name
         ORDER BY n DESC, owner_name
         LIMIT 20",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (name, count) = row?;
        println!("  count={count} owner_name={name:?}");
    }
    Ok(())
}
