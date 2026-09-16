use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_config::{check_config_dir, load, ConfigDirError};
use radar_store::export_ranked_csv;
use rusqlite::Connection;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: export_ranked <destination-db> <config-dir> <csv-path>");
        return ExitCode::from(2);
    }
    let db_path = PathBuf::from(&args[1]);
    let config_dir = PathBuf::from(&args[2]);
    let csv_path = PathBuf::from(&args[3]);

    println!("config directory={}", config_dir.display());
    if let Err(err) = check_config_dir(&config_dir) {
        match err {
            ConfigDirError::MissingDirectory => {
                eprintln!("config directory does not exist: {}", config_dir.display());
            }
            ConfigDirError::MissingFiles(files) => {
                eprintln!(
                    "config directory {} is missing files: {}",
                    config_dir.display(),
                    files.join(", ")
                );
            }
        }
        return ExitCode::from(1);
    }

    let config = match load(&config_dir) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("failed to load config: {err}");
            return ExitCode::from(1);
        }
    };
    let conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("failed to open database: {err}");
            return ExitCode::from(1);
        }
    };

    let ranked = match export_ranked_csv(&config, &conn, &csv_path) {
        Ok(ranked) => ranked,
        Err(err) => {
            eprintln!("export failed: {err}");
            return ExitCode::from(1);
        }
    };

    let working_set = ranked.iter().filter(|row| row.in_working_set).count();
    println!("total rows exported={}", ranked.len());
    println!("working set size={working_set}");
    println!("first 25 rows");
    for row in ranked.iter().take(25) {
        println!(
            "rank={} in_working_set={} display_name={:?} group_key={:?} member_name_count={} total_parcels={:?} improvement_value_zero_count={:?} band_label={:?} band_favourability={:?} municipality_count={} trigger_status={}",
            row.rank,
            row.in_working_set,
            row.display_name,
            row.group_key,
            row.member_name_count,
            row.total_parcels,
            row.improvement_value_zero_count,
            row.band_label,
            row.band_favourability,
            row.municipality_count,
            row.trigger_status
        );
    }

    let mut dist: BTreeMap<String, i64> = BTreeMap::new();
    for row in &ranked {
        let key = match &row.band_favourability {
            Some(value) => format!("{value:?}"),
            None => "none".to_string(),
        };
        *dist.entry(key).or_insert(0) += 1;
    }
    println!("band_favourability distribution");
    for (label, count) in &dist {
        println!("  {label}={count}");
    }

    ExitCode::SUCCESS
}
