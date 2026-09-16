use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use radar_config::{check_config_dir, config_fingerprint, ConfigDirError, SessionState};
use radar_store::{
    insert_outcome, migrate, RecordOutcomeRequest, SUBJECT_ASSIGNMENT_ASSIGNEE, SUBJECT_OWNER_GROUP,
};
use rusqlite::Connection;

const DEFAULT_CONFIG_DIR: &str = "config/dr_data_icp";
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

    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return ExitCode::from(if args.is_empty() { 2 } else { 0 });
    }
    if args.len() < 4 {
        eprintln!("record_outcome needs subject_type, subject_key, action and occurred_on");
        print_usage();
        return ExitCode::from(2);
    }

    let subject_type = args[0].as_str();
    if subject_type != SUBJECT_OWNER_GROUP && subject_type != SUBJECT_ASSIGNMENT_ASSIGNEE {
        eprintln!("subject_type must be {SUBJECT_OWNER_GROUP} or {SUBJECT_ASSIGNMENT_ASSIGNEE}");
        return ExitCode::from(2);
    }

    let config_dir = PathBuf::from(env::var("RADAR_CONFIG_DIR").unwrap_or_else(|_| DEFAULT_CONFIG_DIR.to_string()));
    let db_path = PathBuf::from(env::var("RADAR_DB_PATH").unwrap_or_else(|_| DEFAULT_DB_PATH.to_string()));

    if let Err(err) = check_config_dir(&config_dir) {
        eprintln!("{}", format_config_dir_error(&config_dir, &err));
        return ExitCode::from(1);
    }
    let state = match SessionState::from_dir(&config_dir) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("config load failed: {err}");
            return ExitCode::from(1);
        }
    };
    let config = match state {
        SessionState::Ready { config, .. } => config,
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

    let request = RecordOutcomeRequest {
        subject_type: subject_type.to_string(),
        subject_key: args[1].clone(),
        county_fips: Some(config.manifest.county_fips.clone()),
        action: args[2].clone(),
        occurred_on: Some(args[3].clone()),
        reason: args.get(4).cloned(),
        config_fingerprint: config_fingerprint(&config_dir),
    };

    let mut conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("open {db_path}: {err}", db_path = db_path.display());
            return ExitCode::from(1);
        }
    };
    if let Err(err) = migrate(&mut conn) {
        eprintln!("migrate: {err}");
        return ExitCode::from(1);
    }
    match insert_outcome(&conn, &request) {
        Ok(recorded) => {
            println!(
                "inserted id={} subject_type={} subject_key={} action={} occurred_on={} fingerprint={}",
                recorded.id,
                recorded.subject_type,
                recorded.subject_key,
                recorded.action,
                recorded.occurred_on,
                recorded.config_fingerprint
            );
            ExitCode::from(0)
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn print_usage() {
    println!(
        "Backfill an outcome with a past date. Insert only. Does not rank or score.

cargo run -p radar-store --bin record_outcome -- owner_group \"ACME LLC\" closed_won 2025-04-12 \"existing client\"
cargo run -p radar-store --bin record_outcome -- assignment_assignee 641234/1234/1 closed_lost 2023-11-02 \"lost to bid\"

Arguments: <subject_type> <subject_key> <action> <occurred_on> [reason]
subject_type is owner_group or assignment_assignee.
assignment_assignee subject_key is reel_no/frame_no/sequence.
action is contacted, met, proposed, closed_won, closed_lost, or not_a_fit.
occurred_on is YYYY-MM-DD and may be in the past.
Config directory defaults to {DEFAULT_CONFIG_DIR} or RADAR_CONFIG_DIR.
Database defaults to {DEFAULT_DB_PATH} or RADAR_DB_PATH."
    );
}

fn format_config_dir_error(dir: &Path, err: &ConfigDirError) -> String {
    match err {
        ConfigDirError::MissingDirectory => format!("config directory missing: {}", dir.display()),
        ConfigDirError::MissingFiles(files) => {
            format!("config at {} is missing {}", dir.display(), files.join(", "))
        }
    }
}
