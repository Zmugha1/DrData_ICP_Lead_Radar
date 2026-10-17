use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use radar_store::{
    detect_occupancy_new_business, fetch_and_load_accela_occupancy, migrate,
    newest_stored_record_date, next_poll_start_date, print_accela_load,
};
use rusqlite::Connection;

const DEFAULT_DB_PATH: &str = "data/radar.db";
const REQUIRED_REPO: &str = r"C:\Users\zumah\DrData_ICP_Lead_radar";

fn main() -> ExitCode {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    println!("working directory={}", cwd.display());
    let cwd_s = cwd.to_string_lossy();
    if !cwd_s.eq_ignore_ascii_case(REQUIRED_REPO) {
        eprintln!("STOP: working directory is not {REQUIRED_REPO}");
        return ExitCode::from(2);
    }
    if cwd_s.to_ascii_lowercase().contains("karl-intel-desktop")
        || cwd_s.to_ascii_lowercase().contains("mvp_hub")
    {
        eprintln!("STOP: this is Karl-Intel-Desktop or MVP_Hub");
        return ExitCode::from(2);
    }
    println!("repo confirmed DrData_ICP_Lead_radar (not Karl-Intel-Desktop, not MVP_Hub)");
    println!("two-stage Accela occupancy: session search, then rate-limited CapDetail GET");

    let args: Vec<String> = env::args().collect();
    let mut db_path = PathBuf::from(DEFAULT_DB_PATH);
    let mut since: Option<String> = None;
    let mut end: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--since" => {
                since = args.get(i + 1).cloned();
                i += 2;
            }
            "--end" => {
                end = args.get(i + 1).cloned();
                i += 2;
            }
            other if other.starts_with('-') => {
                eprintln!("unknown flag {other}");
                return ExitCode::from(2);
            }
            other => {
                db_path = PathBuf::from(other);
                i += 1;
            }
        }
    }

    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                eprintln!("create db directory: {err}");
                return ExitCode::from(1);
            }
        }
    }

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

    let start = if let Some(since) = since {
        since
    } else {
        match newest_stored_record_date(&conn) {
            Ok(newest) => match next_poll_start_date(newest.as_deref()) {
                Some(start) => start,
                None => {
                    eprintln!(
                        "STOP: accela_occupancy is empty. Pass --since MM/DD/YYYY for the first poll."
                    );
                    return ExitCode::from(2);
                }
            },
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::from(1);
            }
        }
    };
    println!(
        "poll Start Date={start} End Date={}",
        end.as_deref().unwrap_or("(empty, through today)")
    );

    let fetched_at = fetched_at_now();
    let started = std::time::Instant::now();
    let result =
        match fetch_and_load_accela_occupancy(&mut conn, &start, end.as_deref(), &fetched_at) {
            Ok(result) => result,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::from(1);
            }
        };
    let elapsed = started.elapsed();
    print_accela_load(&result);
    println!(
        "elapsed_ms={} elapsed_s={:.1}",
        elapsed.as_millis(),
        elapsed.as_secs_f64()
    );
    match detect_occupancy_new_business(&conn) {
        Ok(hits) => {
            println!(
                "occupancy_new_business hits={} (original table, insert-only)",
                hits.len()
            );
        }
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    }
    if let Some(load_run_id) = result.load_run_id {
        match radar_store::detect_occupancy_new_business_for_load(&conn, load_run_id) {
            Ok(hits) => {
                println!(
                    "occupancy_new_business snapshot_hits={} load_run_id={} (Reason for applying = 1-Establish a new business or use)",
                    hits.len(),
                    load_run_id
                );
            }
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::from(0)
}

fn fetched_at_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
