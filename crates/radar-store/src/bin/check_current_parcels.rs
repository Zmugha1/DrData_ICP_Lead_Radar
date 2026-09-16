use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_store::{check_current_parcels_agreement, migrate, print_agreement};
use rusqlite::Connection;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: check_current_parcels <database>");
        return ExitCode::from(2);
    }
    let db = PathBuf::from(&args[1]);
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
    match check_current_parcels_agreement(&conn) {
        Ok(reports) => {
            print_agreement(&reports);
            if reports.iter().all(|r| r.agrees()) {
                println!("agreement=pass");
                ExitCode::SUCCESS
            } else {
                println!("agreement=fail");
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("check failed: {err}");
            ExitCode::from(1)
        }
    }
}
