use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_store::{
    detect_improvement_zero_to_value, migrate, print_breakdown, sample_qualifying, Comparability,
};
use rusqlite::Connection;

fn main() -> ExitCode {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let db_path = repo.join("data/radar.db");
    println!("db={}", db_path.display());

    let mut conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("open db: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = migrate(&mut conn) {
        eprintln!("migrate: {err}");
        return ExitCode::from(1);
    }

    println!("==== Waukesha 1 to 16 ====");
    let outcome = match detect_improvement_zero_to_value(&conn, 1, 16) {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("detect: {err}");
            return ExitCode::from(1);
        }
    };
    print_breakdown(&outcome);

    match outcome.comparability {
        Comparability::Comparable => {}
        Comparability::SamePublishedDate { .. } | Comparability::ValuesNotDated => {
            println!("no qualifying sample; pair is not a change observation");
            return ExitCode::from(0);
        }
    }

    if outcome.breakdown.qualifying_transitions == 0 {
        println!("none qualified");
        return ExitCode::from(0);
    }

    println!("==== twenty random qualifying rows ====");
    match sample_qualifying(&conn, 1, 16, 20) {
        Ok(rows) => {
            for (i, row) in rows.iter().enumerate() {
                println!(
                    "{i} taxkey={} municipality={:?} earlier_owner={:?} later_owner={:?} earlier_improvement={} later_improvement={} earlier_value_as_of={:?} later_value_as_of={:?} window_days={:?} later_assessed={:?}",
                    row.taxkey,
                    row.municipality,
                    row.earlier_owner_name,
                    row.later_owner_name,
                    row.earlier_value,
                    row.later_value,
                    row.earlier_value_as_of,
                    row.later_value_as_of,
                    row.window_days,
                    row.later_assessed_value
                );
            }
            if rows.len() < 20 {
                println!(
                    "sampled {} rows; fewer than twenty qualifying transitions",
                    rows.len()
                );
            }
        }
        Err(err) => {
            eprintln!("sample: {err}");
            return ExitCode::from(1);
        }
    }

    let mut by_muni: BTreeMap<String, i64> = BTreeMap::new();
    for row in &outcome.observations {
        let key = row
            .muni_code
            .clone()
            .unwrap_or_else(|| "null muni_code".to_string());
        *by_muni.entry(key).or_insert(0) += 1;
    }
    println!("qualifying count by municipality (muni_code):");
    for (muni, count) in &by_muni {
        let name: Option<String> = conn
            .query_row(
                "SELECT muni_name FROM muni_lookup
                 WHERE muni_code = ?1 AND county_fips = '55133'",
                [muni.as_str()],
                |row| row.get(0),
            )
            .ok();
        println!("  {muni} name={name:?} count={count}");
    }
    if by_muni.len() == 1 {
        println!("all qualifying rows cluster in one municipality");
    }
    println!("qualifying count by window_days:");
    for (window, count) in &outcome.window_days_distribution {
        match window {
            None => println!("  null: {count}"),
            Some(days) => println!("  {days}: {count}"),
        }
    }
    if outcome.window_days_distribution.len() == 1 {
        println!("all qualifying rows share a single window");
    }

    ExitCode::from(0)
}
