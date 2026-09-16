use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_store::detection::liquor::{
    detect_liquor_latest, print_liquor_detections, print_liquor_diff, LiquorComparability,
};
use radar_store::{fetch_and_load_liquor, migrate, snapshot_metas, LIQUOR_CSV_URL};
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
    println!("adapter fetches {LIQUOR_CSV_URL}");

    let args: Vec<String> = env::args().collect();
    let db_path = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(DEFAULT_DB_PATH));
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

    let before = match snapshot_metas(&conn) {
        Ok(metas) => metas,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if let Some(yesterday) = before.last() {
        println!(
            "yesterday snapshot_date={} row_count={}",
            yesterday.snapshot_date, yesterday.row_count
        );
    }

    let (fetch, loaded) = match fetch_and_load_liquor(&mut conn) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    println!("fetch status={}", fetch.status);
    println!(
        "today snapshot_date={} load_run_id={} row_count={} placeholder_rows={} already_loaded={}",
        loaded.snapshot_date,
        loaded.load_run_id,
        loaded.row_count,
        loaded.placeholder_rows,
        loaded.already_loaded
    );
    if let Some(yesterday) = before.last() {
        let delta = loaded.row_count - yesterday.row_count;
        println!(
            "row_count today={} yesterday={} delta={}",
            loaded.row_count, yesterday.row_count, delta
        );
        if loaded.row_count == yesterday.row_count {
            println!("row_count identical to yesterday {}", yesterday.row_count);
        }
    }

    let diff = match detect_liquor_latest(&conn) {
        Ok(diff) => diff,
        Err(err) => {
            eprintln!("detect: {err}");
            return ExitCode::from(1);
        }
    };
    print_liquor_diff(&diff);
    match diff.comparability {
        LiquorComparability::Comparable => {}
        LiquorComparability::SameSnapshotDate { .. }
        | LiquorComparability::InsufficientSnapshots { .. } => {
            eprintln!("comparability_guard=not passed");
            return ExitCode::from(1);
        }
    }
    if let Err(err) = print_liquor_detections(&conn, &diff) {
        eprintln!("print detections: {err}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
