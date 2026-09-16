// Downloads only URLs a human pasted into urls.txt. This is not a
// crawler. It never discovers attachment ids and never requests
// Application Search.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::StoreError;
use crate::liquor::LIQUOR_USER_AGENT;

pub const LIRA_PDF_FOLDER: &str =
    r"C:\Users\zumah\Desktop\radar_sources\milwaukee_lira_pdfs";
pub const LIRA_PDF_URLS_NAME: &str = "urls.txt";
pub const LIRA_PDF_ALLOWED_HOST: &str = "itmdapps.milwaukee.gov";
pub const LIRA_PDF_ALLOWED_PATH: &str = "applattachservlet";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfFetchStatus {
    Downloaded,
    SkippedExisting,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFetchRow {
    pub url: String,
    pub status: PdfFetchStatus,
    pub bytes: Option<u64>,
    pub saved_filename: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFetchResult {
    pub folder: PathBuf,
    pub urls_path: PathBuf,
    pub rows: Vec<PdfFetchRow>,
    pub urls_missing: bool,
}

pub fn lira_pdf_urls_path(folder: &Path) -> PathBuf {
    folder.join(LIRA_PDF_URLS_NAME)
}

pub fn prepare_lira_pdf_folder(folder: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(folder).map_err(|err| {
        StoreError::LiraPdf(format!("create {}: {err}", folder.display()))
    })
}

pub fn parse_url_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
}

pub fn application_id_from_url(url: &str) -> Option<String> {
    let bytes = url.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let slice = url.get(i..).unwrap_or("");
        let lower = slice.to_ascii_lowercase();
        if lower.starts_with("?id=") || lower.starts_with("&id=") {
            let rest = &slice[4..];
            let end = rest.find('&').unwrap_or(rest.len());
            let id = rest[..end].trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
        i += 1;
    }
    None
}

pub fn safe_pdf_filename(url: &str) -> String {
    if let Some(id) = application_id_from_url(url) {
        let safe: String = id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        if !safe.is_empty() {
            return format!("{safe}.pdf");
        }
    }
    let mut digest = 0u64;
    for byte in url.as_bytes() {
        digest = digest.wrapping_mul(16777619).wrapping_add(*byte as u64);
    }
    format!("attachment_{digest:016x}.pdf")
}

pub fn url_is_allowed_attachment(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("url is not http or https".to_string());
    }
    let without_scheme = if lower.starts_with("https://") {
        &trimmed[8..]
    } else {
        &trimmed[7..]
    };
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    if !host.eq_ignore_ascii_case(LIRA_PDF_ALLOWED_HOST) {
        return Err(format!(
            "host {host} is not {LIRA_PDF_ALLOWED_HOST}; refusing to fetch"
        ));
    }
    if !lower.contains(LIRA_PDF_ALLOWED_PATH) {
        return Err("path is not ApplAttachServlet; refusing to fetch".to_string());
    }
    Ok(())
}

pub trait PdfHttp {
    fn get_bytes(&self, url: &str) -> Result<(u16, Vec<u8>), String>;
}

pub struct LivePdfHttp;

impl PdfHttp for LivePdfHttp {
    fn get_bytes(&self, url: &str) -> Result<(u16, Vec<u8>), String> {
        match ureq::get(url)
            .set("User-Agent", LIQUOR_USER_AGENT)
            .timeout(Duration::from_secs(60))
            .call()
        {
            Ok(response) => {
                let status = response.status();
                let mut body = Vec::new();
                response
                    .into_reader()
                    .read_to_end(&mut body)
                    .map_err(|err| err.to_string())?;
                Ok((status, body))
            }
            Err(ureq::Error::Status(status, response)) => {
                let mut body = Vec::new();
                let _ = response.into_reader().read_to_end(&mut body);
                Ok((status, body))
            }
            Err(err) => Err(err.to_string()),
        }
    }
}

fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.len() >= 5 && &bytes[..5] == b"%PDF-"
}

fn fetch_once(http: &dyn PdfHttp, url: &str) -> Result<(u16, Vec<u8>), String> {
    http.get_bytes(url)
}

pub fn fetch_pdfs_from_url_file(
    folder: &Path,
    http: &dyn PdfHttp,
) -> Result<PdfFetchResult, StoreError> {
    prepare_lira_pdf_folder(folder)?;
    let urls_path = lira_pdf_urls_path(folder);
    if !urls_path.is_file() {
        return Ok(PdfFetchResult {
            folder: folder.to_path_buf(),
            urls_path,
            rows: Vec::new(),
            urls_missing: true,
        });
    }
    let text = fs::read_to_string(&urls_path).map_err(|err| {
        StoreError::LiraPdf(format!("read {}: {err}", urls_path.display()))
    })?;
    let urls = parse_url_list(&text);
    let mut rows = Vec::new();
    for url in urls {
        match url_is_allowed_attachment(&url) {
            Ok(()) => {}
            Err(reason) => {
                rows.push(PdfFetchRow {
                    url,
                    status: PdfFetchStatus::Failed,
                    bytes: None,
                    saved_filename: None,
                    reason: Some(reason),
                });
                continue;
            }
        }
        let filename = safe_pdf_filename(&url);
        let dest = folder.join(&filename);
        if dest.is_file() {
            let bytes = dest.metadata().ok().map(|m| m.len());
            rows.push(PdfFetchRow {
                url,
                status: PdfFetchStatus::SkippedExisting,
                bytes,
                saved_filename: Some(filename),
                reason: Some("already downloaded".to_string()),
            });
            continue;
        }
        let mut last_reason: Option<String> = None;
        let mut last_bytes: Option<u64> = None;
        let mut downloaded: Option<Vec<u8>> = None;
        for _attempt in 0..2 {
            match fetch_once(http, &url) {
                Ok((status, bytes)) if status == 200 && looks_like_pdf(&bytes) => {
                    downloaded = Some(bytes);
                    break;
                }
                Ok((status, bytes)) => {
                    last_bytes = Some(bytes.len() as u64);
                    last_reason = Some(if status != 200 {
                        format!("status={status}")
                    } else {
                        format!(
                            "status={status} body is not a PDF ({} bytes)",
                            bytes.len()
                        )
                    });
                }
                Err(err) => last_reason = Some(err),
            }
        }
        if let Some(bytes) = downloaded {
            fs::write(&dest, &bytes).map_err(|err| {
                StoreError::LiraPdf(format!("write {}: {err}", dest.display()))
            })?;
            rows.push(PdfFetchRow {
                url,
                status: PdfFetchStatus::Downloaded,
                bytes: Some(bytes.len() as u64),
                saved_filename: Some(filename),
                reason: None,
            });
        } else {
            rows.push(PdfFetchRow {
                url,
                status: PdfFetchStatus::Failed,
                bytes: last_bytes,
                saved_filename: None,
                reason: Some(format!(
                    "{} after two attempts",
                    last_reason.unwrap_or_else(|| "request failed".to_string())
                )),
            });
        }
    }
    Ok(PdfFetchResult {
        folder: folder.to_path_buf(),
        urls_path,
        rows,
        urls_missing: false,
    })
}

pub fn print_pdf_fetch(result: &PdfFetchResult) {
    if result.urls_missing {
        println!(
            "urls.txt missing. Expected location: {}. Create that file with one ApplAttachServlet URL per line, pasted by a human, then rerun. This tool does not crawl itmdapps.milwaukee.gov.",
            result.urls_path.display()
        );
        return;
    }
    println!("lira pdf folder={}", result.folder.display());
    println!("urls file={}", result.urls_path.display());
    if result.rows.is_empty() {
        println!("url count=0");
        return;
    }
    for row in &result.rows {
        let status = match row.status {
            PdfFetchStatus::Downloaded => "downloaded",
            PdfFetchStatus::SkippedExisting => "skipped_existing",
            PdfFetchStatus::Failed => "failed",
        };
        println!(
            "url={} status={} bytes={} filename={} reason={}",
            row.url,
            status,
            row.bytes.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string()),
            row.saved_filename.as_deref().unwrap_or("-"),
            row.reason.as_deref().unwrap_or("-")
        );
    }
}

pub fn list_lira_pdfs(folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let mut paths = Vec::new();
    if !folder.is_dir() {
        return Ok(paths);
    }
    let entries = fs::read_dir(folder).map_err(|err| {
        StoreError::LiraPdf(format!("read {}: {err}", folder.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            StoreError::LiraPdf(format!("read {}: {err}", folder.display()))
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("pdf")) == Some(true)
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct SeqHttp {
        results: std::cell::RefCell<Vec<Result<(u16, Vec<u8>), String>>>,
    }

    impl PdfHttp for SeqHttp {
        fn get_bytes(&self, _url: &str) -> Result<(u16, Vec<u8>), String> {
            let mut q = self.results.borrow_mut();
            if q.is_empty() {
                Err("no more responses".to_string())
            } else {
                q.remove(0)
            }
        }
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("radar_lira_pdf_{nanos}"));
        fs::create_dir_all(&dir).expect("dir");
        dir
    }

    #[test]
    fn parse_skips_blank_and_comment() {
        let text = "# note\n\nhttps://itmdapps.milwaukee.gov/LiraPublic/ApplAttachServlet?id=99\n";
        assert_eq!(parse_url_list(text).len(), 1);
    }

    #[test]
    fn application_id_and_filename() {
        let url = "https://itmdapps.milwaukee.gov/LiraPublic/ApplAttachServlet?id=12345";
        assert_eq!(application_id_from_url(url).as_deref(), Some("12345"));
        assert_eq!(safe_pdf_filename(url), "12345.pdf");
    }

    #[test]
    fn rejects_non_attachment_url() {
        let err = url_is_allowed_attachment(
            "https://itmdapps.milwaukee.gov/LiraPublic/applicationsearch.jsp",
        )
        .unwrap_err();
        assert!(err.contains("ApplAttachServlet"));
    }

    #[test]
    fn missing_urls_file_does_not_fetch() {
        let dir = temp_dir();
        let http = SeqHttp {
            results: std::cell::RefCell::new(vec![]),
        };
        let result = fetch_pdfs_from_url_file(&dir, &http).expect("run");
        assert!(result.urls_missing);
        assert!(result.rows.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_existing_and_does_not_call_http() {
        let dir = temp_dir();
        let url = "https://itmdapps.milwaukee.gov/LiraPublic/ApplAttachServlet?id=77";
        fs::write(dir.join("urls.txt"), format!("{url}\n")).expect("urls");
        fs::write(dir.join("77.pdf"), b"%PDF-1.4 already").expect("pdf");
        let http = SeqHttp {
            results: std::cell::RefCell::new(vec![Err("should not be called".to_string())]),
        };
        let result = fetch_pdfs_from_url_file(&dir, &http).expect("run");
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].status, PdfFetchStatus::SkippedExisting);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn two_attempts_then_skip() {
        let dir = temp_dir();
        let url = "https://itmdapps.milwaukee.gov/LiraPublic/ApplAttachServlet?id=88";
        fs::write(dir.join("urls.txt"), format!("{url}\n")).expect("urls");
        let http = SeqHttp {
            results: std::cell::RefCell::new(vec![
                Err("timeout".to_string()),
                Err("timeout".to_string()),
            ]),
        };
        let result = fetch_pdfs_from_url_file(&dir, &http).expect("run");
        assert_eq!(result.rows[0].status, PdfFetchStatus::Failed);
        assert!(result.rows[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("two attempts"));
        assert!(!dir.join("88.pdf").exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn successful_download_writes_pdf() {
        let dir = temp_dir();
        let url = "https://itmdapps.milwaukee.gov/LiraPublic/ApplAttachServlet?id=55";
        fs::write(dir.join("urls.txt"), format!("{url}\n")).expect("urls");
        let mut body = b"%PDF-1.4 test".to_vec();
        body.extend_from_slice(&[0u8; 8]);
        let http = SeqHttp {
            results: std::cell::RefCell::new(vec![Ok((200, body.clone()))]),
        };
        let result = fetch_pdfs_from_url_file(&dir, &http).expect("run");
        assert_eq!(result.rows[0].status, PdfFetchStatus::Downloaded);
        assert_eq!(fs::read(dir.join("55.pdf")).expect("read"), body);
        fs::remove_dir_all(dir).ok();
    }
}
