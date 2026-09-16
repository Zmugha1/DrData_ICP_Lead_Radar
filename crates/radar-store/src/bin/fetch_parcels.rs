use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use radar_config::{
    check_config_dir, ConfigDirError, FieldMap, Finding, RadarConfig, SessionState,
    REQUIRED_CONFIG_FILES,
};
use radar_store::{
    compare_load_runs, compare_load_runs_opts, load_parcels, migrate, print_current_view_check,
    print_latest_load_run, print_load_run, LiveParcelSource,
};
use rusqlite::{params, Connection};

const DEFAULT_CONFIG_DIR: &str = "config/dr_data_icp";
const DEFAULT_DB_PATH: &str = "data/radar.db";

const PLAN_COMMISSION_KEYS: &[&str] = &[
    "PWV 0901984006",
    "PWV 0896017",
    "PWV 0896018",
    "PWV 0897993",
    "PWV 0900061",
    "PWC 0900064",
    "PWC 0953984007",
    "PWC 0953984008",
    "PWC 0912983",
    "PWC 0912984",
    "PWC 0912985",
    "BR C1141999005",
    "BR C1116995001",
    "BR C1091001001",
    "BR C1091002",
    "DELC0793020002",
    "DELC0803988004",
    "DELC0788008001",
    "DELC0788011",
    "DELT 0839.025",
    "DELT0860.991.001",
    "DELT0859.997",
    "MRTT 0349-992-002",
    "MRTT 0346-993",
    "MRTT 0362-991-001",
    "GNT 1551.999.003",
    "GNT 1579-011",
    "LSBT 0257.999.002",
    "LSBT 0224.999.005",
    "LSBT 0151.997",
    "LSBT 0284.995.021",
    "LSBT 0155.995",
    "2204.046",
    "2204.040",
    "2204.041",
    "2204.045",
    "2162.996.002",
    "2171.113",
    "2260.998.002",
    "2211.996.001",
    "2206.998.040",
    "2229.992.006",
    "2173.998",
    "2232.998",
    "2273.990.002",
];

struct Cli {
    compare_only: bool,
    load_only: bool,
    config_dir: PathBuf,
    config_used_default: bool,
    db_path: PathBuf,
}

fn parse_cli(args: &[String]) -> Cli {
    let mut compare_only = false;
    let mut load_only = false;
    let mut positionals: Vec<&str> = Vec::new();
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--compare-only" => compare_only = true,
            "--load-only" => load_only = true,
            other => positionals.push(other),
        }
    }
    if compare_only {
        let db_path = if positionals.is_empty() {
            PathBuf::from(DEFAULT_DB_PATH)
        } else {
            PathBuf::from(positionals[0])
        };
        return Cli {
            compare_only: true,
            load_only: false,
            config_dir: PathBuf::from(DEFAULT_CONFIG_DIR),
            config_used_default: true,
            db_path,
        };
    }
    let (config_dir, config_used_default) = match positionals.first() {
        Some(path) => (PathBuf::from(path), false),
        None => (PathBuf::from(DEFAULT_CONFIG_DIR), true),
    };
    let db_path = if positionals.len() >= 2 {
        PathBuf::from(positionals[1])
    } else {
        PathBuf::from(DEFAULT_DB_PATH)
    };
    Cli {
        compare_only: false,
        load_only,
        config_dir,
        config_used_default,
        db_path,
    }
}

fn selection_lines(cli: &Cli) -> Vec<String> {
    let mut lines = Vec::new();
    if cli.config_used_default {
        lines.push(format!(
            "config directory default used: {DEFAULT_CONFIG_DIR}"
        ));
    }
    lines.push(format!("config directory={}", cli.config_dir.display()));
    lines
}

fn format_config_dir_error(dir: &Path, err: &ConfigDirError) -> String {
    match err {
        ConfigDirError::MissingDirectory => {
            format!("config directory does not exist: {}", dir.display())
        }
        ConfigDirError::MissingFiles(files) => {
            format!(
                "config directory {} is missing files: {}",
                dir.display(),
                files.join(", ")
            )
        }
    }
}

fn print_findings(findings: &[Finding]) {
    if findings.is_empty() {
        println!("warnings: none");
        return;
    }
    println!("warnings ({})", findings.len());
    for finding in findings {
        println!(
            "  {}: {} [{}]",
            finding.rule_code, finding.message, finding.location
        );
    }
}

fn print_optional_field(name: &str, value: &Option<String>) {
    match value {
        Some(text) => println!("    {name}={text}"),
        None => println!("    {name}=<absent>"),
    }
}

fn print_field_map(field_map: &FieldMap) {
    println!("    taxkey={}", field_map.taxkey);
    println!("    muni_code={}", field_map.muni_code);
    println!("    owner_name={}", field_map.owner_name);
    print_optional_field("site_address", &field_map.site_address);
    print_optional_field("mail_address_1", &field_map.mail_address_1);
    print_optional_field("mail_address_2", &field_map.mail_address_2);
    print_optional_field("assessed_value", &field_map.assessed_value);
    print_optional_field("improvement_value", &field_map.improvement_value);
    print_optional_field("land_value", &field_map.land_value);
    print_optional_field("prop_class", &field_map.prop_class);
    print_optional_field("rec_source", &field_map.rec_source);
}

fn print_loaded_config(path: &Path, config: &RadarConfig, warnings: &[Finding]) {
    println!("config files loaded={}/{}", REQUIRED_CONFIG_FILES.len(), REQUIRED_CONFIG_FILES.len());
    println!("config session=Ready");
    println!("county_fips={}", config.entity.county_fips);
    print_findings(warnings);
    println!("layers ({})", config.entity.layers.len());
    for layer in &config.entity.layers {
        println!("  role={:?} url={}", layer.role, layer.url);
        println!("  field_map:");
        print_field_map(&layer.field_map);
    }
    println!("spine_source_id={}", config.entity.spine_source_id);
    match &config.entity.lookup_source {
        Some(radar_config::LookupSource::Layer {
            url,
            where_clause,
            code_field,
            name_field,
        }) => {
            println!(
                "lookup_source kind=Layer url={url} where={where_clause} code_field={code_field} name_field={name_field}"
            );
        }
        Some(radar_config::LookupSource::Derived {
            code_field,
            name_field,
        }) => {
            println!(
                "lookup_source kind=Derived code_field={code_field} name_field={name_field}"
            );
        }
        None => println!("lookup_source=<absent>"),
    }
    println!(
        "E1/E2 would query muni_lookup WHERE county_fips='{}'",
        config.entity.county_fips
    );
    println!("resolved path={}", path.display());
}

fn fail_config_dir(dir: &Path, err: &ConfigDirError) -> ExitCode {
    eprintln!("{}", format_config_dir_error(dir, err));
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let cli = parse_cli(&args);

    if cli.compare_only {
        let db_path = cli.db_path;
        let mut conn = match Connection::open(&db_path) {
            Ok(conn) => conn,
            Err(err) => {
                eprintln!("failed to open {}: {err}", db_path.display());
                return ExitCode::from(1);
            }
        };
        if let Err(err) = migrate(&mut conn) {
            eprintln!("failed to migrate: {err}");
            return ExitCode::from(1);
        }
        println!("database={}", db_path.display());
        if let Err(err) = print_load_run(&conn, 5) {
            eprintln!("failed to print load_run 5: {err}");
            return ExitCode::from(1);
        }
        println!("=== compare run 1 vs run 5 ===");
        if let Err(err) = compare_load_runs(&conn, 1, 5) {
            eprintln!("failed to compare load runs: {err}");
            return ExitCode::from(1);
        }
        println!("=== current_entities ===");
        if let Err(err) = print_current_view_check(&conn) {
            eprintln!("failed current_entities check: {err}");
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }

    for line in selection_lines(&cli) {
        println!("{line}");
    }
    if let Err(err) = check_config_dir(&cli.config_dir) {
        return fail_config_dir(&cli.config_dir, &err);
    }

    let state = match SessionState::from_dir(&cli.config_dir) {
        Ok(state) => state,
        Err(err) => {
            eprintln!(
                "failed to load config from {}: {err}",
                cli.config_dir.display()
            );
            return ExitCode::from(1);
        }
    };
    let config = match state {
        SessionState::Ready {
            config,
            warnings,
            path,
        } => {
            if cli.load_only {
                print_loaded_config(&path, &config, &warnings);
                return ExitCode::SUCCESS;
            }
            config
        }
        SessionState::Invalid { path, findings } => {
            eprintln!("config at {} is not Ready", path.display());
            for finding in findings {
                eprintln!(
                    "{}: {} [{}]",
                    finding.rule_code, finding.message, finding.location
                );
            }
            return ExitCode::from(1);
        }
        SessionState::Unconfigured => {
            eprintln!("config is Unconfigured");
            return ExitCode::from(1);
        }
    };

    let db_path = cli.db_path;
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                eprintln!("failed to create database directory: {err}");
                return ExitCode::from(1);
            }
        }
    }

    let mut conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("failed to open {}: {err}", db_path.display());
            return ExitCode::from(1);
        }
    };
    if let Err(err) = migrate(&mut conn) {
        eprintln!("failed to migrate: {err}");
        return ExitCode::from(1);
    }

    println!("config directory={}", cli.config_dir.display());
    println!("resolved path={}", cli.config_dir.display());
    println!("collapse_duplicate_geometry={}", config.entity.collapse_duplicate_geometry);
    println!("layers ({})", config.entity.layers.len());
    for layer in &config.entity.layers {
        println!("  role={:?} url={}", layer.role, layer.url);
    }
    match &config.entity.lookup_source {
        Some(radar_config::LookupSource::Layer {
            url,
            where_clause,
            code_field,
            name_field,
        }) => println!(
            "lookup_source kind=Layer url={url} where={where_clause} code={code_field} name={name_field}"
        ),
        Some(radar_config::LookupSource::Derived {
            code_field,
            name_field,
        }) => println!(
            "lookup_source kind=Derived code_field={code_field} name_field={name_field}"
        ),
        None => println!("lookup_source=<absent>"),
    }

    let fetched_at = now_rfc3339();
    println!("fetched_at={fetched_at}");
    println!("database={}", db_path.display());

    let source = LiveParcelSource::new();
    let primary_url = config
        .entity
        .primary_layer()
        .map(|layer| layer.url.as_str())
        .unwrap_or("");
    if let Err(err) = print_task1_field_dump(&source, primary_url) {
        eprintln!("Task 1 field dump failed (continuing to load): {err}");
    }
    match load_parcels(&mut conn, &source, &fetched_at, &config.entity) {
        Ok(result) => {
            println!(
                "load success id={} written={} skipped={} requested={} pass1={} pass2={} pass2_excluded={} pages={} live_page_size={} historic_page_size={} taxkeys_normalised={} pass1_norm={} pass2_norm={} measured_lag_days={:?} live_lag_days={:?} historic_lag_days={:?} fallback=[{}] uncovered=[{}] live_export_date=[{}] historic_export_date=[{}] cross_pass_duplicate_taxkeys={}",
                result.load_run_id,
                result.records_written,
                result.records_skipped,
                result.records_requested,
                result.pass1_count,
                result.pass2_count,
                result.pass2_excluded,
                result.pages_fetched,
                result.page_size,
                result.historic_page_size,
                result.taxkeys_normalised,
                result.taxkeys_normalised_pass1,
                result.taxkeys_normalised_pass2,
                result.measured_lag_days,
                result.live_lag_days,
                result.historic_lag_days,
                result.fallback_codes.join(","),
                result.uncovered_codes.join(","),
                result.live_export_dates.join(","),
                result.historic_export_dates.join(","),
                result.cross_pass_duplicate_taxkeys
            );
            if let Err(err) = print_load_run(&conn, result.load_run_id) {
                eprintln!("failed to print load_run: {err}");
                return ExitCode::from(1);
            }
            if result.fallback_codes.is_empty() {
                println!("no municipality falls back under where=1=1");
                println!("run is single-pass live only");
                match historic_count_run5(&conn) {
                    Ok(n) => println!(
                        "the {n} historic_year_end rows from load_run 5 are not fetched in this run"
                    ),
                    Err(err) => eprintln!("failed to count load_run 5 historic rows: {err}"),
                }
            }
            println!("=== compare run 11 vs run {} ===", result.load_run_id);
            println!("=== load_run 11 ===");
            if let Err(err) = print_load_run(&conn, 11) {
                eprintln!("failed to print load_run 11: {err}");
                return ExitCode::from(1);
            }
            println!("=== load_run {} ===", result.load_run_id);
            if let Err(err) = print_load_run(&conn, result.load_run_id) {
                eprintln!("failed to print load_run {}: {err}", result.load_run_id);
                return ExitCode::from(1);
            }
            if let Err(err) = print_run_parity(&conn, 11, result.load_run_id) {
                eprintln!("failed run 11 vs {} parity: {err}", result.load_run_id);
                return ExitCode::from(1);
            }
            if let Err(err) = compare_load_runs_opts(&conn, 11, result.load_run_id, false) {
                eprintln!("failed to compare load runs: {err}");
                return ExitCode::from(1);
            }
            println!("=== plan commission parcel keys (space strip only) ===");
            if let Err(err) = print_plan_commission_match(&conn) {
                eprintln!("failed plan commission match: {err}");
                return ExitCode::from(1);
            }
            println!("=== Task 6 current_entities ===");
            if let Err(err) = print_current_view_check(&conn) {
                eprintln!("failed current_entities check: {err}");
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("load failed: {err}");
            let _ = print_latest_load_run(&conn);
            ExitCode::from(1)
        }
    }
}

fn now_rfc3339() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock");
    let secs = now.as_secs() as i64;
    let nanos = now.subsec_nanos();
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let (y, m, d) = civil_ymd(days);
    let hour = sod / 3600;
    let min = (sod % 3600) / 60;
    let sec = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}.{nanos:09}+00:00")
}

fn civil_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 }.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m as u32, d as u32)
}

fn print_task1_field_dump(source: &LiveParcelSource, layer_url: &str) -> Result<(), radar_store::StoreError> {
    println!("=== TASK 1 LIVE LAYER FIELD DUMP ===");
    dump_one_taxkey(source, layer_url, "TL", "TAXKEY='TL'")?;
    for value in ["GAP", "OVERLAP", "WAKCPENDING", "COMMON", "CITY"] {
        let clause = format!("TAXKEY='{value}'");
        let n = dump_one_taxkey(source, layer_url, value, &clause)?;
        if n == 0 {
            let upper = format!("UPPER(TAXKEY)='{value}'");
            dump_one_taxkey(source, layer_url, value, &upper)?;
        }
    }
    Ok(())
}

fn dump_one_taxkey(
    source: &LiveParcelSource,
    layer_url: &str,
    label: &str,
    where_clause: &str,
) -> Result<usize, radar_store::StoreError> {
    println!("--- {label} where={where_clause} ---");
    let rows = source.query_all_attributes(layer_url, where_clause)?;
    if rows.is_empty() {
        println!("no features returned");
        return Ok(0);
    }
    for attrs in &rows {
        println!("{}", serde_json::to_string_pretty(attrs).unwrap_or_else(|_| attrs.to_string()));
        println!(
            "fields: municipality(MUNI)={:?} owner(OWNERNAMEFULL)={:?} site(SITEADRESS)={:?} prop_class(PROPCLASS)={:?} aux_class(AUXCLASS)={:?}",
            json_attr(attrs, "MUNI"),
            json_attr(attrs, "OWNERNAMEFULL"),
            json_attr(attrs, "SITEADRESS"),
            json_attr(attrs, "PROPCLASS"),
            json_attr(attrs, "AUXCLASS"),
        );
        if label == "TL" {
            println!(
                "The TL record does not explain what TL stands for; no field expands the acronym."
            );
        }
    }
    Ok(rows.len())
}

fn json_attr(attrs: &serde_json::Value, key: &str) -> serde_json::Value {
    attrs
        .get(key)
        .cloned()
        .or_else(|| attrs.get(&key.to_ascii_lowercase()).cloned())
        .unwrap_or(serde_json::Value::Null)
}

fn print_run_parity(conn: &Connection, run_a: i64, run_b: i64) -> Result<(), rusqlite::Error> {
    println!("=== TASK 5 SIDE BY SIDE run {run_a} vs run {run_b} ===");
    let written_a: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1",
        [run_a],
        |row| row.get(0),
    )?;
    let written_b: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1",
        [run_b],
        |row| row.get(0),
    )?;
    let skipped_a: Option<i64> = conn.query_row(
        "SELECT records_skipped FROM load_runs WHERE id = ?1",
        [run_a],
        |row| row.get(0),
    )?;
    let skipped_b: Option<i64> = conn.query_row(
        "SELECT records_skipped FROM load_runs WHERE id = ?1",
        [run_b],
        |row| row.get(0),
    )?;
    let error_a: Option<String> = conn.query_row(
        "SELECT error_text FROM load_runs WHERE id = ?1",
        [run_a],
        |row| row.get(0),
    )?;
    let error_b: Option<String> = conn.query_row(
        "SELECT error_text FROM load_runs WHERE id = ?1",
        [run_b],
        |row| row.get(0),
    )?;
    let munis_a: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT muni_code) FROM entities WHERE load_run_id = ?1 AND muni_code IS NOT NULL",
        [run_a],
        |row| row.get(0),
    )?;
    let munis_b: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT muni_code) FROM entities WHERE load_run_id = ?1 AND muni_code IS NOT NULL",
        [run_b],
        |row| row.get(0),
    )?;
    let owner_a: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND owner_name IS NULL",
        [run_a],
        |row| row.get(0),
    )?;
    let owner_b: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND owner_name IS NULL",
        [run_b],
        |row| row.get(0),
    )?;
    let imp_a: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND improvement_value IS NULL",
        [run_a],
        |row| row.get(0),
    )?;
    let imp_b: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND improvement_value IS NULL",
        [run_b],
        |row| row.get(0),
    )?;
    let prop_a: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT prop_class) FROM entities WHERE load_run_id = ?1",
        [run_a],
        |row| row.get(0),
    )?;
    let prop_b: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT prop_class) FROM entities WHERE load_run_id = ?1",
        [run_b],
        |row| row.get(0),
    )?;
    println!("{:<28} {:>12} {:>12} {:>8}", "metric", run_a, run_b, "match");
    let rows = [
        ("written", written_a, written_b),
        ("skipped", skipped_a.unwrap_or(-1), skipped_b.unwrap_or(-1)),
        ("distinct muni_code", munis_a, munis_b),
        ("owner_name null", owner_a, owner_b),
        ("improvement_value null", imp_a, imp_b),
        ("distinct prop_class", prop_a, prop_b),
    ];
    let mut any_diff = false;
    for (label, a, b) in rows {
        let match_text = if a == b {
            "yes"
        } else {
            any_diff = true;
            "REGRESSION"
        };
        println!("{label:<28} {a:>12} {b:>12} {match_text:>8}");
    }
    println!("error_text run {run_a}={error_a:?}");
    println!("error_text run {run_b}={error_b:?}");
    if error_a != error_b {
        any_diff = true;
        println!("error_text REGRESSION");
    } else {
        println!("error_text match=yes");
    }
    if any_diff {
        println!("TASK 5 RESULT: regression; run {run_a} and run {run_b} differ");
    } else {
        println!("TASK 5 RESULT: match; config move is proven safe on the known result");
    }
    Ok(())
}

fn historic_count_run5(conn: &Connection) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM entities
         WHERE load_run_id = 5 AND value_source = 'historic_year_end'",
        [],
        |row| row.get(0),
    )
}

fn print_plan_commission_match(conn: &Connection) -> Result<(), rusqlite::Error> {
    let total = PLAN_COMMISSION_KEYS.len();
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT taxkey FROM current_entities WHERE REPLACE(taxkey, ' ', '') = ?1 LIMIT 1",
    )?;
    for key in PLAN_COMMISSION_KEYS {
        let stripped: String = key.chars().filter(|c| *c != ' ').collect();
        let hit = stmt.query_row(params![stripped], |row| row.get::<_, String>(0));
        match hit {
            Ok(store_key) => matched.push((*key, store_key)),
            Err(rusqlite::Error::QueryReturnedNoRows) => unmatched.push(*key),
            Err(err) => return Err(err),
        }
    }
    println!(
        "plan commission keys resolving under space strip={}/{}",
        matched.len(),
        total
    );
    for (raw, store) in &matched {
        println!("  matched packet={raw} current_entities={store}");
    }
    println!("still-unmatched keys={}", unmatched.len());
    for key in &unmatched {
        println!("  unmatched {key}");
    }
    let christian = "PWV0901984006";
    let christian_resolves = matched.iter().any(|(raw, store)| {
        raw.chars().filter(|c| *c != ' ').collect::<String>() == christian
            || store.chars().filter(|c| *c != ' ').collect::<String>() == christian
    });
    println!(
        "PWV0901984006, the Christian Brothers parcel, resolves={}",
        if christian_resolves { "yes" } else { "no" }
    );

    let store_keys: Vec<String> = {
        let mut stmt = conn.prepare("SELECT taxkey FROM current_entities")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row?);
        }
        keys
    };
    let punct_store: std::collections::HashSet<String> = store_keys
        .iter()
        .map(|key| key.chars().filter(|c| c.is_ascii_alphanumeric()).collect())
        .collect();
    let punct_matched = PLAN_COMMISSION_KEYS
        .iter()
        .filter(|key| {
            let stripped: String = key.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            punct_store.contains(&stripped)
        })
        .count();
    println!(
        "plan commission keys that would resolve if punctuation were also stripped={punct_matched}/{}",
        PLAN_COMMISSION_KEYS.len()
    );

    let muskego: Vec<&&str> = PLAN_COMMISSION_KEYS
        .iter()
        .filter(|key| {
            key.chars()
                .find(|c| !c.is_whitespace())
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        })
        .collect();
    let space_store: std::collections::HashSet<String> = store_keys
        .iter()
        .map(|key| key.chars().filter(|c| *c != ' ').collect())
        .collect();
    let muskego_prefixed = muskego
        .iter()
        .filter(|key| {
            let stripped: String = key.chars().filter(|c| *c != ' ').collect();
            space_store.contains(&format!("MSKC{stripped}"))
        })
        .count();
    println!(
        "Muskego keys that would resolve if MSKC were prepended={muskego_prefixed}/{}",
        muskego.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "radar_fetch_parcels_{label}_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn missing_directory_fails_before_any_network_call() {
        let dir = PathBuf::from("config/does_not_exist_fetch_parcels_selection");
        let err = check_config_dir(&dir).expect_err("missing directory must fail");
        assert_eq!(err, ConfigDirError::MissingDirectory);
        let message = format_config_dir_error(&dir, &err);
        assert!(
            message.contains("does_not_exist_fetch_parcels_selection"),
            "got {message}"
        );
        assert!(
            message.contains("does not exist"),
            "got {message}"
        );
    }

    #[test]
    fn directory_missing_one_config_file_fails_and_names_it() {
        let dir = unique_temp("missing_one");
        for name in REQUIRED_CONFIG_FILES {
            if name == "entity.toml" {
                continue;
            }
            fs::write(dir.join(name), "").expect("touch config file");
        }
        let err = check_config_dir(&dir).expect_err("incomplete directory must fail");
        match err {
            ConfigDirError::MissingFiles(files) => {
                assert_eq!(files, vec!["entity.toml".to_string()]);
            }
            other => panic!("expected MissingFiles, got {other:?}"),
        }
        let message = format_config_dir_error(
            &dir,
            &check_config_dir(&dir).expect_err("still missing"),
        );
        assert!(
            message.contains("entity.toml"),
            "missing file must be named, got {message}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_argument_resolves_to_the_default_and_prints_it() {
        let cli = parse_cli(&[String::from("fetch_parcels")]);
        assert!(cli.config_used_default);
        assert_eq!(cli.config_dir, PathBuf::from(DEFAULT_CONFIG_DIR));
        let lines = selection_lines(&cli);
        assert_eq!(
            lines[0],
            format!("config directory default used: {DEFAULT_CONFIG_DIR}")
        );
        assert_eq!(
            lines[1],
            format!("config directory={DEFAULT_CONFIG_DIR}")
        );
    }

    #[test]
    fn explicit_argument_resolves_to_that_path() {
        let cli = parse_cli(&[
            String::from("fetch_parcels"),
            String::from("config/milwaukee_parcels"),
        ]);
        assert!(!cli.config_used_default);
        assert_eq!(cli.config_dir, PathBuf::from("config/milwaukee_parcels"));
        let lines = selection_lines(&cli);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "config directory=config/milwaukee_parcels");
    }
}

