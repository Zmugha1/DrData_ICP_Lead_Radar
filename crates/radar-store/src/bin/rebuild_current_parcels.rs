use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use radar_store::{migrate, rebuild_current_parcels};
use rusqlite::Connection;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("usage: rebuild_current_parcels <database> [county_fips]");
        return ExitCode::from(2);
    }
    let db = PathBuf::from(&args[1]);
    let county = args.get(2).map(String::as_str);
    let mut conn = match Connection::open(&db) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("failed to open {}: {err}", db.display());
            return ExitCode::from(1);
        }
    };
    if let Err(err) = migrate(&mut conn) {
        eprintln!("failed to migrate: {err}");
        return ExitCode::from(1);
    }
    let started = Instant::now();
    match rebuild_current_parcels(&conn, county) {
        Ok(n) => {
            println!(
                "rebuilt current_parcels rows={n} county={} elapsed={:?}",
                county.unwrap_or("all"),
                started.elapsed()
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("rebuild failed: {err}");
            ExitCode::from(1)
        }
    }
}
