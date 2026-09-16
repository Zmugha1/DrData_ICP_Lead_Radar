use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_store::{
    fetch_pdfs_from_url_file, prepare_lira_pdf_folder, print_pdf_fetch, LivePdfHttp,
    LIRA_PDF_FOLDER,
};

const REQUIRED_REPO: &str = r"C:\Users\zumah\DrData_ICP_Lead_radar";

fn main() -> ExitCode {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    println!("working directory={}", cwd.display());
    let cwd_s = cwd.to_string_lossy();
    if !cwd_s.eq_ignore_ascii_case(REQUIRED_REPO) {
        eprintln!("STOP: working directory is not {REQUIRED_REPO}");
        return ExitCode::from(2);
    }
    println!("repo confirmed DrData_ICP_Lead_radar");
    println!("this binary fetches only URLs pasted in urls.txt; it does not crawl itmdapps.milwaukee.gov");

    let args: Vec<String> = env::args().collect();
    let folder = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(LIRA_PDF_FOLDER));
    if let Err(err) = prepare_lira_pdf_folder(&folder) {
        eprintln!("{err}");
        return ExitCode::from(1);
    }
    match fetch_pdfs_from_url_file(&folder, &LivePdfHttp) {
        Ok(result) => {
            print_pdf_fetch(&result);
            if result.urls_missing {
                return ExitCode::from(0);
            }
            if result
                .rows
                .iter()
                .any(|row| matches!(row.status, radar_store::lira_pdf::PdfFetchStatus::Failed))
            {
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
