use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_store::{
    anthropic_api_key_from_env, anthropic_model_from_env, extract_folder, fetch_pdfs_from_url_file,
    format_validation_report, list_lira_pdfs, migrate, print_pdf_fetch, AnthropicClient,
    LivePdfHttp, ANTHROPIC_API_KEY_ABSENT, LIRA_PDF_FOLDER,
};
use rusqlite::Connection;

const DEFAULT_DB_PATH: &str = "data/radar.db";
const REQUIRED_REPO: &str = r"C:\Users\zumah\DrData_ICP_Lead_radar";
const REPORT_PATH: &str = "docs/research/ICP_EXTRACTION_REPORT.md";

fn main() -> ExitCode {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    println!("working directory={}", cwd.display());
    let cwd_s = cwd.to_string_lossy();
    if !cwd_s.eq_ignore_ascii_case(REQUIRED_REPO) {
        eprintln!("STOP: working directory is not {REQUIRED_REPO}");
        return ExitCode::from(2);
    }
    println!("repo confirmed DrData_ICP_Lead_radar");

    let args: Vec<String> = env::args().collect();
    let db_path = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(DEFAULT_DB_PATH));
    let folder = PathBuf::from(args.get(2).map(String::as_str).unwrap_or(LIRA_PDF_FOLDER));

    let fetch = match fetch_pdfs_from_url_file(&folder, &LivePdfHttp) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    print_pdf_fetch(&fetch);
    let fetch_note = if fetch.urls_missing {
        format!(
            "urls.txt was missing at {}. The downloader did not request itmdapps.milwaukee.gov.",
            fetch.urls_path.display()
        )
    } else {
        format!(
            "url rows={}. Downloaded only pasted ApplAttachServlet URLs.",
            fetch.rows.len()
        )
    };

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

    let pdfs = match list_lira_pdfs(&folder) {
        Ok(pdfs) => pdfs,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    println!("pdf count={}", pdfs.len());

    let mut records = Vec::new();
    let mut hand_notes: Vec<String> = Vec::new();
    if pdfs.is_empty() {
        hand_notes.push(
            "No application PDF was on disk. Hand comparison against the form was not possible."
                .to_string(),
        );
    } else {
        match anthropic_api_key_from_env() {
            Some(key) => {
                println!("ANTHROPIC_API_KEY=set length={}", key.len());
                let client = match AnthropicClient::from_env() {
                    Ok(client) => client,
                    Err(err) => {
                        eprintln!("{err}");
                        return ExitCode::from(2);
                    }
                };
                let model = anthropic_model_from_env();
                println!("ANTHROPIC_MODEL={model}");
                let script = cwd.join("scripts").join("rasterize_pdf.py");
                match extract_folder(&mut conn, &client, &model, &folder, &script) {
                    Ok(got) => records = got,
                    Err(err) => {
                        eprintln!("{err}");
                        let report = format_validation_report(&records, &fetch_note, &hand_notes);
                        let _ = fs::create_dir_all("docs/research");
                        let _ = fs::write(REPORT_PATH, report);
                        return ExitCode::from(1);
                    }
                }
                hand_notes.push(
                    "Extracted values are listed above. This run is a demonstration. Compare each non-null field to the scan before trusting volume."
                        .to_string(),
                );
            }
            None => {
                eprintln!("{ANTHROPIC_API_KEY_ABSENT}");
                hand_notes.push(
                    "ANTHROPIC_API_KEY was absent. PDFs were not sent to the model.".to_string(),
                );
                let report = format_validation_report(&records, &fetch_note, &hand_notes);
                if let Err(err) = write_report(&report) {
                    eprintln!("{err}");
                    return ExitCode::from(1);
                }
                println!("wrote {REPORT_PATH} lines={}", report.lines().count());
                return ExitCode::from(2);
            }
        }
    }

    let report = format_validation_report(&records, &fetch_note, &hand_notes);
    if let Err(err) = write_report(&report) {
        eprintln!("{err}");
        return ExitCode::from(1);
    }
    println!("wrote {REPORT_PATH} lines={}", report.lines().count());
    for record in &records {
        println!(
            "extracted file={} unreadable={}",
            record.application_source_file,
            record.fields_unreadable.join(",")
        );
    }
    ExitCode::SUCCESS
}

fn write_report(report: &str) -> Result<(), String> {
    fs::create_dir_all("docs/research").map_err(|err| err.to_string())?;
    fs::write(REPORT_PATH, report).map_err(|err| err.to_string())
}
