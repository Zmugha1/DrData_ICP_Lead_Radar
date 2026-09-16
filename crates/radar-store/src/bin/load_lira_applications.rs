use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_store::{
    load_lira_from_folder, migrate, non_renewal_rows, prepare_lira_folder, print_lira_load,
    print_non_renewal_rows, type_counts, LIRA_APPLICATIONS_FOLDER,
};
use radar_store::lira::{print_empty_folder, print_type_counts};
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
    println!("loader reads local CSV only; it does not fetch itmdapps.milwaukee.gov");

    let args: Vec<String> = env::args().collect();
    let db_path = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(DEFAULT_DB_PATH));
    let folder = PathBuf::from(
        args.get(2)
            .map(String::as_str)
            .unwrap_or(LIRA_APPLICATIONS_FOLDER),
    );

    if let Err(err) = prepare_lira_folder(&folder) {
        eprintln!("{err}");
        return ExitCode::from(1);
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

    let result = match load_lira_from_folder(&mut conn, &folder) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if result.empty_folder {
        print_empty_folder(&folder);
        return ExitCode::from(0);
    }

    print_lira_load(&result);
    match type_counts(&conn) {
        Ok(counts) => print_type_counts(&counts),
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    }
    match non_renewal_rows(&conn) {
        Ok(rows) => print_non_renewal_rows(&rows),
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}
