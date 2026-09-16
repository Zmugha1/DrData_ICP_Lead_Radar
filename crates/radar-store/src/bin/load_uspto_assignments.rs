use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use radar_config::{check_config_dir, ConfigDirError, SessionState, REQUIRED_CONFIG_FILES};
use radar_store::{
    list_assignment_zips, load_assignments_from_folder, migrate, prepare_assignments_folder,
    USPTO_DOWNLOAD_PAGE,
};
use rusqlite::Connection;

const DEFAULT_CONFIG_DIR: &str = "config/dr_data_icp";
const DEFAULT_DB_PATH: &str = "data/radar.db";
const DEFAULT_FOLDER: &str = r"C:\Users\zumah\Desktop\radar_sources\uspto_assignments";
const DEFAULT_DOWNLOADS: &str = r"C:\Users\zumah\Downloads";
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

    let args: Vec<String> = env::args().collect();
    let config_dir = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(DEFAULT_CONFIG_DIR));
    let db_path = PathBuf::from(args.get(2).map(String::as_str).unwrap_or(DEFAULT_DB_PATH));
    let folder = PathBuf::from(args.get(3).map(String::as_str).unwrap_or(DEFAULT_FOLDER));
    let downloads = PathBuf::from(args.get(4).map(String::as_str).unwrap_or(DEFAULT_DOWNLOADS));

    match prepare_assignments_folder(&folder, &downloads) {
        Ok(moved) => {
            if moved.is_empty() {
                println!("no ad*.zip moved from {}", downloads.display());
            } else {
                println!("moved {} zip(s) into {}", moved.len(), folder.display());
            }
        }
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    }

    let zips = match list_assignment_zips(&folder) {
        Ok(zips) => zips,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if zips.is_empty() {
        println!(
            "assignment folder is empty: {}. Download daily adYYYYMMDD.zip files from {USPTO_DOWNLOAD_PAGE} and place them in that folder.",
            folder.display()
        );
        return ExitCode::from(0);
    }

    println!(
        "config: added twelfth required file assignments.toml in config/dr_data_icp and config/milwaukee_parcels. State values and the Waukesha and Milwaukee city lists live there, not in code. County is not stored."
    );

    if let Err(err) = check_config_dir(&config_dir) {
        eprintln!("{}", format_config_dir_error(&config_dir, &err));
        return ExitCode::from(1);
    }
    println!(
        "config files loaded={}/{}",
        REQUIRED_CONFIG_FILES.len(),
        REQUIRED_CONFIG_FILES.len()
    );

    let state = match SessionState::from_dir(&config_dir) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("config load failed: {err}");
            return ExitCode::from(1);
        }
    };
    let config = match state {
        SessionState::Ready { config, warnings, .. } => {
            for warning in warnings {
                println!("config warning {}: {}", warning.rule_code, warning.message);
            }
            config
        }
        SessionState::Invalid { findings, .. } => {
            for finding in findings {
                eprintln!(
                    "config {}: {} ({})",
                    finding.rule_code, finding.message, finding.location
                );
            }
            return ExitCode::from(1);
        }
        SessionState::Unconfigured => {
            eprintln!("config unconfigured");
            return ExitCode::from(1);
        }
    };

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

    match load_assignments_from_folder(&mut conn, &folder, &config.assignments) {
        Ok(result) => {
            if result.empty_folder {
                return ExitCode::from(0);
            }
            println!(
                "uspto assignments load_run_id={}",
                result.load_run_id.unwrap_or(0)
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn format_config_dir_error(dir: &Path, err: &ConfigDirError) -> String {
    match err {
        ConfigDirError::MissingDirectory => {
            format!("config directory {} does not exist", dir.display())
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
