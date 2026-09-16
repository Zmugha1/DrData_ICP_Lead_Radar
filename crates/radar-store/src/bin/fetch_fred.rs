use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use radar_config::{check_config_dir, ConfigDirError, SessionState, REQUIRED_CONFIG_FILES};
use radar_store::{
    fred_api_key_from_env, load_fred_series, migrate, print_load_run, LiveFredClient,
    FRED_API_KEY_ABSENT,
};
use rusqlite::Connection;

const DEFAULT_CONFIG_DIR: &str = "config/dr_data_icp";
const DEFAULT_DB_PATH: &str = "data/radar.db";

fn main() -> ExitCode {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    println!("working directory={}", cwd.display());

    match fred_api_key_from_env() {
        Some(key) => {
            println!("FRED_API_KEY=set length={}", key.len());
        }
        None => {
            eprintln!("{FRED_API_KEY_ABSENT}");
            return ExitCode::from(2);
        }
    }

    let args: Vec<String> = env::args().collect();
    let mut positionals: Vec<&str> = Vec::new();
    for arg in args.iter().skip(1) {
        positionals.push(arg);
    }
    let config_dir = PathBuf::from(positionals.first().copied().unwrap_or(DEFAULT_CONFIG_DIR));
    let db_path = PathBuf::from(positionals.get(1).copied().unwrap_or(DEFAULT_DB_PATH));

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

    let client = match LiveFredClient::from_env() {
        Ok(client) => client,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
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

    match load_fred_series(&mut conn, &config.series, &client) {
        Ok(result) => {
            println!("fred load_run_id={}", result.load_run_id);
            if let Err(err) = print_load_run(&conn, result.load_run_id) {
                eprintln!("print load run: {err}");
                return ExitCode::from(1);
            }
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
