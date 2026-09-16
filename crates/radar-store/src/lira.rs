// LIRA Application Search CSVs. A human exports them from
// itmdapps.milwaukee.gov with the page's own Export to CSV control.
// This loader never fetches that host. It never opens PDFs or
// ApplAttachServlet. Owner, agent, and partner columns are not in
// the schema; if a future export contains one, the column is skipped
// and the skip is reported.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::StoreError;
use crate::liquor::{decode_csv_bytes, parse_csv};

pub const LIRA_SOURCE_ID: &str = "src-milwaukee-lira-applications";
pub const LIRA_APPLICATIONS_FOLDER: &str =
    r"C:\Users\zumah\Desktop\radar_sources\milwaukee_lira";
pub const LIRA_EXPORT_URL_PATTERN: &str =
    "ReportServletLocal?folderPath=/ITMD/WebApps/LiraPublic/&q=App_By_LicenseType&lType=<code>&format=CSV";
pub const LIRA_LICENSE_TYPE_CODES: &[&str] = &["301", "221", "19", "262", "8", "523"];

pub const TYPE_RENEWAL: &str = "RENEWAL";
pub const TYPE_APPLICATION: &str = "APPLICATION";
pub const TYPE_TRANSFER_BUSINESS_OPS: &str = "Transfer - Change of Business Operations";
pub const TYPE_TRANSFER_AGENT_OFFICER: &str = "Transfer - Change of Agent, Officer";
pub const TYPE_TRANSFER_NAME: &str = "Transfer - Name change";
pub const TYPE_TRANSFER_LOCATION: &str = "Transfer - Change of Location";

pub const KNOWN_LICENSE_TYPES: &[&str] = &[
    TYPE_RENEWAL,
    TYPE_APPLICATION,
    TYPE_TRANSFER_BUSINESS_OPS,
    TYPE_TRANSFER_AGENT_OFFICER,
    TYPE_TRANSFER_NAME,
    TYPE_TRANSFER_LOCATION,
];

const HTML_LICENSE_TITLE: &str = "License Title";
const HTML_BUSINESS_NAME: &str = "Business Name";
const HTML_ADDRESS: &str = "Address";
const HTML_TYPE: &str = "Type";
const HTML_APPLICATION_DATE: &str = "Application Date";
const HTML_ATTACHMENTS: &str = "Attachments";

const SSRS_LICENSE_TITLE: &str = "PROFESSION_FULL_NAME";
const SSRS_BUSINESS_NAME: &str = "LICENSEE";
const SSRS_ADDRESS: &str = "LICENSEE_ADDR";
const SSRS_TYPE: &str = "TYPE";
const SSRS_APPLICATION_DATE: &str = "APPLICATION_DATE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiraHeaderSet {
    HtmlTable,
    Ssrs,
    Mixed,
    NoneMatched,
}

impl LiraHeaderSet {
    pub fn as_label(self) -> &'static str {
        match self {
            LiraHeaderSet::HtmlTable => "html table",
            LiraHeaderSet::Ssrs => "ssrs",
            LiraHeaderSet::Mixed => "mixed html table and ssrs",
            LiraHeaderSet::NoneMatched => "none",
        }
    }
}

fn mapped_field(header: &str) -> Option<&'static str> {
    match header {
        HTML_LICENSE_TITLE | SSRS_LICENSE_TITLE => Some("license_title"),
        HTML_BUSINESS_NAME | SSRS_BUSINESS_NAME => Some("business_name"),
        HTML_ADDRESS | SSRS_ADDRESS => Some("address"),
        HTML_TYPE | SSRS_TYPE => Some("license_type_column"),
        HTML_APPLICATION_DATE | SSRS_APPLICATION_DATE => Some("application_date"),
        HTML_ATTACHMENTS => Some("has_attachment"),
        _ => None,
    }
}

fn is_html_header(header: &str) -> bool {
    matches!(
        header,
        HTML_LICENSE_TITLE
            | HTML_BUSINESS_NAME
            | HTML_ADDRESS
            | HTML_TYPE
            | HTML_APPLICATION_DATE
            | HTML_ATTACHMENTS
    )
}

fn is_ssrs_header(header: &str) -> bool {
    matches!(
        header,
        SSRS_LICENSE_TITLE | SSRS_BUSINESS_NAME | SSRS_ADDRESS | SSRS_TYPE | SSRS_APPLICATION_DATE
    )
}

fn classify_header_set(mapped_headers: &[String]) -> LiraHeaderSet {
    let html = mapped_headers.iter().any(|h| is_html_header(h));
    let ssrs = mapped_headers.iter().any(|h| is_ssrs_header(h));
    match (html, ssrs) {
        (true, false) => LiraHeaderSet::HtmlTable,
        (false, true) => LiraHeaderSet::Ssrs,
        (true, true) => LiraHeaderSet::Mixed,
        (false, false) => LiraHeaderSet::NoneMatched,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiraRow {
    pub license_title: Option<String>,
    pub business_name: Option<String>,
    pub address: Option<String>,
    pub license_type_column: Option<String>,
    pub application_date: Option<String>,
    pub has_attachment: Option<bool>,
    pub unrecognized_type: bool,
    pub unparsed_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiraFileStats {
    pub source_file: String,
    pub skipped: bool,
    pub stored: i64,
    pub unexpected_columns: Vec<String>,
    pub skipped_owner_columns: Vec<String>,
    pub unrecognized_types: Vec<String>,
    pub header_set: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiraLoadResult {
    pub load_run_id: Option<i64>,
    pub empty_folder: bool,
    pub folder: PathBuf,
    pub files: Vec<LiraFileStats>,
    pub skipped_files: i64,
    pub stored_rows: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiraTypeCounts {
    pub application: i64,
    pub transfer_business_ops: i64,
    pub transfer_agent_officer: i64,
    pub transfer_name_change: i64,
    pub transfer_location: i64,
    pub renewal: i64,
    pub other: BTreeMap<String, i64>,
}

fn lira_err(msg: String) -> StoreError {
    StoreError::Lira(msg)
}

pub fn prepare_lira_folder(folder: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(folder).map_err(|err| {
        lira_err(format!("create {}: {err}", folder.display()))
    })
}

pub fn list_lira_csvs(folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
    if !folder.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    let entries = fs::read_dir(folder).map_err(|err| {
        lira_err(format!("read {}: {err}", folder.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            lira_err(format!("read {}: {err}", folder.display()))
        })?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("csv"))
                .unwrap_or(false)
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn lira_source_file_key(path: &Path) -> Result<String, StoreError> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.csv");
    let meta = fs::metadata(path).map_err(|err| {
        lira_err(format!("stat {}: {err}", path.display()))
    })?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(format!("{name}|mtime_unix={mtime}|bytes={}", meta.len()))
}

fn source_file_already_loaded(conn: &Connection, source_file: &str) -> Result<bool, StoreError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM lira_applications WHERE source_file = ?1 LIMIT 1",
            [source_file],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn forbidden_column_reason(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    if n.contains("date of birth")
        || n.contains("birth date")
        || n == "dob"
        || n.contains("birthdate")
    {
        return Some("date of birth");
    }
    if n.contains("driver") {
        return Some("driver's license");
    }
    if n.contains("social security") || n.contains("ssn") {
        return Some("ssn");
    }
    if n.contains("personal phone") {
        return Some("personal phone");
    }
    if n.contains("personal email") {
        return Some("personal email");
    }
    if n.contains("owner") {
        return Some("owner");
    }
    if n.contains("partner") {
        return Some("partner");
    }
    if n.contains("agent") {
        return Some("agent");
    }
    None
}

fn header_key(name: &str) -> String {
    name.trim().trim_start_matches('\u{feff}').to_string()
}

fn is_known_type(value: &str) -> bool {
    KNOWN_LICENSE_TYPES.iter().any(|known| *known == value)
}

fn parse_has_attachment(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "yes" | "y" | "true" | "1" => Some(true),
        "no" | "n" | "false" | "0" => Some(false),
        "" => None,
        _ => None,
    }
}

fn parse_application_date(raw: &str) -> Result<String, ()> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(());
    }
    if s.len() >= 10 && s.as_bytes()[4] == b'-' {
        if chrono_ymd(&s[..10]).is_some() {
            return Ok(s[..10].to_string());
        }
    }
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() == 3 {
        let m: u32 = parts[0].parse().map_err(|_| ())?;
        let d: u32 = parts[1].parse().map_err(|_| ())?;
        let y: i32 = parts[2].parse().map_err(|_| ())?;
        if (1..=12).contains(&m) && (1..=31).contains(&d) && (1900..=2100).contains(&y) {
            return Ok(format!("{y:04}-{m:02}-{d:02}"));
        }
    }
    Err(())
}

fn chrono_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    if (1..=12).contains(&m) && (1..=31).contains(&d) {
        Some((y, m, d))
    } else {
        None
    }
}

pub fn parse_lira_csv(
    text: &str,
) -> Result<(Vec<LiraRow>, Vec<String>, Vec<String>, LiraHeaderSet), StoreError> {
    let records = parse_csv(text).map_err(|err| match err {
        StoreError::Liquor(msg) => lira_err(msg.replace("liquor csv", "lira csv")),
        other => other,
    })?;
    if records.is_empty() {
        return Err(lira_err("lira csv has no header".to_string()));
    }
    let header: Vec<String> = records[0].iter().map(|h| header_key(h)).collect();
    let mut unexpected = Vec::new();
    let mut skipped_owner = Vec::new();
    let mut index: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut mapped_headers = Vec::new();
    for (i, name) in header.iter().enumerate() {
        if name.is_empty() {
            unexpected.push("(empty header)".to_string());
            continue;
        }
        if let Some(reason) = forbidden_column_reason(name) {
            skipped_owner.push(format!("{name} ({reason})"));
            continue;
        }
        if let Some(field) = mapped_field(name) {
            index.insert(field, i);
            mapped_headers.push(name.clone());
        } else {
            unexpected.push(name.clone());
        }
    }
    let header_set = classify_header_set(&mapped_headers);
    let mut rows = Vec::new();
    for record in records.iter().skip(1) {
        if record.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let get = |field: &str| -> Option<String> {
            index
                .get(field)
                .and_then(|i| record.get(*i))
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let type_raw = get("license_type_column");
        let unrecognized_type = match type_raw.as_deref() {
            None => true,
            Some(value) => !is_known_type(value),
        };
        let date_raw = get("application_date");
        let (application_date, unparsed_date) = match date_raw.as_deref() {
            None => (None, None),
            Some(raw) => match parse_application_date(raw) {
                Ok(iso) => (Some(iso), None),
                Err(()) => (Some(raw.to_string()), Some(raw.to_string())),
            },
        };
        rows.push(LiraRow {
            license_title: get("license_title"),
            business_name: get("business_name"),
            address: get("address"),
            license_type_column: type_raw,
            application_date,
            has_attachment: get("has_attachment")
                .as_deref()
                .and_then(parse_has_attachment),
            unrecognized_type,
            unparsed_date,
        });
    }
    Ok((rows, unexpected, skipped_owner, header_set))
}

pub fn load_lira_from_folder(
    conn: &mut Connection,
    folder: &Path,
) -> Result<LiraLoadResult, StoreError> {
    let paths = list_lira_csvs(folder)?;
    if paths.is_empty() {
        return Ok(LiraLoadResult {
            load_run_id: None,
            empty_folder: true,
            folder: folder.to_path_buf(),
            files: Vec::new(),
            skipped_files: 0,
            stored_rows: 0,
        });
    }
    let fetched_at: String =
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
            row.get(0)
        })?;
    let mut files = Vec::new();
    let mut skipped_files = 0i64;
    let mut pending: Vec<(
        String,
        Vec<LiraRow>,
        Vec<String>,
        Vec<String>,
        Vec<String>,
        LiraHeaderSet,
    )> = Vec::new();
    for path in &paths {
        let source_file = lira_source_file_key(path)?;
        if source_file_already_loaded(conn, &source_file)? {
            skipped_files += 1;
            files.push(LiraFileStats {
                source_file,
                skipped: true,
                stored: 0,
                unexpected_columns: Vec::new(),
                skipped_owner_columns: Vec::new(),
                unrecognized_types: Vec::new(),
                header_set: String::new(),
            });
            continue;
        }
        let bytes = fs::read(path).map_err(|err| {
            lira_err(format!("read {}: {err}", path.display()))
        })?;
        let text = decode_csv_bytes(&bytes);
        let (rows, unexpected, skipped_owner, header_set) = parse_lira_csv(&text)?;
        let unrecognized_types: Vec<String> = rows
            .iter()
            .filter(|row| row.unrecognized_type)
            .map(|row| {
                row.license_type_column
                    .clone()
                    .unwrap_or_else(|| "(empty)".to_string())
            })
            .collect();
        pending.push((
            source_file,
            rows,
            unexpected,
            skipped_owner,
            unrecognized_types,
            header_set,
        ));
    }
    let stored_rows: i64 = pending
        .iter()
        .map(|(_, rows, _, _, _, _)| rows.len() as i64)
        .sum();
    let mut load_run_id = None;
    if stored_rows > 0 {
        conn.execute(
            "INSERT INTO load_runs (source_id, started_at, finished_at, status, records_requested, records_written, records_skipped)
             VALUES (?1, ?2, ?2, 'success', ?3, ?3, ?4)",
            params![LIRA_SOURCE_ID, fetched_at, stored_rows, skipped_files],
        )?;
        load_run_id = Some(conn.last_insert_rowid());
        let run_id = load_run_id.expect("run");
        let tx = conn.transaction()?;
        for (source_file, rows, _, _, _, _) in &pending {
            for (index, row) in rows.iter().enumerate() {
                tx.execute(
                    "INSERT INTO lira_applications (
                         source_file, row_index, load_run_id, fetched_at,
                         license_title, business_name, address, license_type_column,
                         application_date, has_attachment
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        source_file,
                        index as i64,
                        run_id,
                        fetched_at,
                        row.license_title,
                        row.business_name,
                        row.address,
                        row.license_type_column,
                        row.application_date,
                        row.has_attachment.map(|v| if v { 1i64 } else { 0i64 }),
                    ],
                )?;
            }
        }
        tx.commit()?;
    }
    for (source_file, rows, unexpected, skipped_owner, unrecognized_types, header_set) in pending
    {
        files.push(LiraFileStats {
            source_file,
            skipped: false,
            stored: rows.len() as i64,
            unexpected_columns: unexpected,
            skipped_owner_columns: skipped_owner,
            unrecognized_types,
            header_set: header_set.as_label().to_string(),
        });
    }
    Ok(LiraLoadResult {
        load_run_id,
        empty_folder: false,
        folder: folder.to_path_buf(),
        files,
        skipped_files,
        stored_rows,
    })
}

pub fn type_counts(conn: &Connection) -> Result<LiraTypeCounts, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT ifnull(license_type_column, ''), COUNT(*)
         FROM lira_applications
         WHERE license_type_column IS NOT NULL OR license_title IS NOT NULL
         GROUP BY 1",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    let mut counts = LiraTypeCounts {
        application: 0,
        transfer_business_ops: 0,
        transfer_agent_officer: 0,
        transfer_name_change: 0,
        transfer_location: 0,
        renewal: 0,
        other: BTreeMap::new(),
    };
    for row in rows {
        let (label, n) = row?;
        match label.as_str() {
            TYPE_APPLICATION => counts.application = n,
            TYPE_TRANSFER_BUSINESS_OPS => counts.transfer_business_ops = n,
            TYPE_TRANSFER_AGENT_OFFICER => counts.transfer_agent_officer = n,
            TYPE_TRANSFER_NAME => counts.transfer_name_change = n,
            TYPE_TRANSFER_LOCATION => counts.transfer_location = n,
            TYPE_RENEWAL => counts.renewal = n,
            other => {
                counts.other.insert(other.to_string(), n);
            }
        }
    }
    Ok(counts)
}

pub fn non_renewal_rows(conn: &Connection) -> Result<Vec<LiraRow>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT license_title, business_name, address, license_type_column,
                application_date, has_attachment
         FROM lira_non_renewal_applications
         ORDER BY application_date DESC, business_name COLLATE NOCASE, address",
    )?;
    let rows = stmt.query_map([], |row| {
        let has: Option<i64> = row.get(5)?;
        let license_type_column: Option<String> = row.get(3)?;
        let unrecognized_type = match license_type_column.as_deref() {
            None => true,
            Some(value) => !is_known_type(value),
        };
        Ok(LiraRow {
            license_title: row.get(0)?,
            business_name: row.get(1)?,
            address: row.get(2)?,
            license_type_column,
            application_date: row.get(4)?,
            has_attachment: has.map(|v| v != 0),
            unrecognized_type,
            unparsed_date: None,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn print_empty_folder(folder: &Path) {
    println!(
        "lira folder is empty: {}. Export CSVs by hand from Application Search using the page Export to CSV control. Do not fetch itmdapps from this loader.",
        folder.display()
    );
    println!("export URL pattern: {LIRA_EXPORT_URL_PATTERN}");
    println!("known license type codes:");
    for code in LIRA_LICENSE_TYPE_CODES {
        println!("  {code}");
    }
}

pub fn print_lira_load(result: &LiraLoadResult) {
    println!("lira folder={}", result.folder.display());
    println!(
        "load_run_id={} skipped_files={} stored_rows={}",
        result.load_run_id.unwrap_or(0),
        result.skipped_files,
        result.stored_rows
    );
    for file in &result.files {
        if file.skipped {
            println!("skipped already loaded source_file={}", file.source_file);
            continue;
        }
        println!(
            "loaded source_file={} rows={} header_set={}",
            file.source_file, file.stored, file.header_set
        );
        for col in &file.unexpected_columns {
            println!("unexpected column in {}: {col}", file.source_file);
        }
        for col in &file.skipped_owner_columns {
            println!(
                "skipped owner/agent/partner column in {}: {col}",
                file.source_file
            );
        }
        for value in &file.unrecognized_types {
            println!(
                "unrecognized Type stored as-is in {}: {value}",
                file.source_file
            );
        }
    }
}

pub fn print_type_counts(counts: &LiraTypeCounts) {
    println!("APPLICATION count={}", counts.application);
    println!(
        "Transfer - Change of Business Operations count={}",
        counts.transfer_business_ops
    );
    println!(
        "Transfer - Change of Agent, Officer count={}",
        counts.transfer_agent_officer
    );
    println!("Transfer - Name change count={}", counts.transfer_name_change);
    println!(
        "Transfer - Change of Location count={}",
        counts.transfer_location
    );
    println!("RENEWAL count={} (reference, not the deliverable)", counts.renewal);
    for (label, n) in &counts.other {
        println!("other Type {label} count={n}");
    }
}

pub fn print_non_renewal_rows(rows: &[LiraRow]) {
    println!("non_renewal_rows={}", rows.len());
    for (i, row) in rows.iter().enumerate() {
        println!(
            "non_renewal {}/{} license_title={} business_name={} address={} type={} application_date={}",
            i + 1,
            rows.len(),
            row.license_title.as_deref().unwrap_or(""),
            row.business_name.as_deref().unwrap_or(""),
            row.address.as_deref().unwrap_or(""),
            row.license_type_column.as_deref().unwrap_or(""),
            row.application_date.as_deref().unwrap_or("")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::migrate;
    use rusqlite::Connection;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("radar_lira_{nanos}"));
        fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn write_csv(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).expect("write csv");
    }

    const HEADER: &str = "License Title,Business Name,Address,Type,Application Date,Attachments\n";

    fn row(title: &str, biz: &str, addr: &str, typ: &str, date: &str) -> String {
        format!("\"{title}\",\"{biz}\",\"{addr}\",\"{typ}\",{date},Yes\n")
    }

    #[test]
    fn renewal_is_excluded_from_the_signal_view() {
        let dir = temp_dir();
        write_csv(
            &dir,
            "one.csv",
            &format!(
                "{HEADER}{}{}",
                row("Food Dealer - Restaurant", "New Co", "1 Main", TYPE_APPLICATION, "09/10/2026"),
                row("Food Dealer - Restaurant", "Old Co", "2 Main", TYPE_RENEWAL, "09/09/2026"),
            ),
        );
        let mut conn = open();
        load_lira_from_folder(&mut conn, &dir).expect("load");
        let signal: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM lira_non_renewal_applications",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(signal, 1);
        let stored: i64 = conn
            .query_row("SELECT COUNT(*) FROM lira_applications", [], |r| r.get(0))
            .expect("all");
        assert_eq!(stored, 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn four_transfer_subtypes_are_counted_separately() {
        let dir = temp_dir();
        write_csv(
            &dir,
            "transfers.csv",
            &format!(
                "{HEADER}{}{}{}{}{}",
                row("T", "A", "1 St", TYPE_TRANSFER_BUSINESS_OPS, "09/01/2026"),
                row("T", "B", "2 St", TYPE_TRANSFER_AGENT_OFFICER, "09/02/2026"),
                row("T", "C", "3 St", TYPE_TRANSFER_NAME, "09/03/2026"),
                row("T", "D", "4 St", TYPE_TRANSFER_LOCATION, "09/04/2026"),
                row("T", "E", "5 St", TYPE_APPLICATION, "09/05/2026"),
            ),
        );
        let mut conn = open();
        load_lira_from_folder(&mut conn, &dir).expect("load");
        let counts = type_counts(&conn).expect("counts");
        assert_eq!(counts.transfer_business_ops, 1);
        assert_eq!(counts.transfer_agent_officer, 1);
        assert_eq!(counts.transfer_name_change, 1);
        assert_eq!(counts.transfer_location, 1);
        assert_eq!(counts.application, 1);
        assert_eq!(counts.renewal, 0);
        let merged = counts.transfer_business_ops
            + counts.transfer_agent_officer
            + counts.transfer_name_change
            + counts.transfer_location;
        assert_eq!(merged, 4);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rerun_on_already_loaded_file_is_skipped() {
        let dir = temp_dir();
        write_csv(
            &dir,
            "same.csv",
            &format!(
                "{HEADER}{}",
                row("T", "Biz", "1 St", TYPE_APPLICATION, "09/10/2026")
            ),
        );
        let mut conn = open();
        let first = load_lira_from_folder(&mut conn, &dir).expect("first");
        assert_eq!(first.stored_rows, 1);
        assert_eq!(first.skipped_files, 0);
        let count_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM lira_applications", [], |r| r.get(0))
            .expect("before");
        let second = load_lira_from_folder(&mut conn, &dir).expect("second");
        assert_eq!(second.stored_rows, 0);
        assert_eq!(second.skipped_files, 1);
        assert!(second.files.iter().all(|f| f.skipped));
        let count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM lira_applications", [], |r| r.get(0))
            .expect("after");
        assert_eq!(count_before, count_after);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unrecognized_type_is_stored_and_flagged_not_dropped() {
        let dir = temp_dir();
        write_csv(
            &dir,
            "odd.csv",
            &format!(
                "{HEADER}{}",
                row("T", "Biz", "1 St", "SOMETHING ELSE", "09/10/2026")
            ),
        );
        let mut conn = open();
        let result = load_lira_from_folder(&mut conn, &dir).expect("load");
        assert_eq!(result.stored_rows, 1);
        assert_eq!(
            result.files[0].unrecognized_types,
            vec!["SOMETHING ELSE".to_string()]
        );
        let stored: String = conn
            .query_row(
                "SELECT license_type_column FROM lira_applications",
                [],
                |r| r.get(0),
            )
            .expect("type");
        assert_eq!(stored, "SOMETHING ELSE");
        let signal: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM lira_non_renewal_applications",
                [],
                |r| r.get(0),
            )
            .expect("view");
        assert_eq!(signal, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn owner_column_is_skipped_and_reported() {
        let csv = "License Title,Business Name,Address,Type,Application Date,Attachments,Owner Name\nFood,Biz,1 St,APPLICATION,09/10/2026,No,SECRET PERSON\n";
        let (rows, unexpected, skipped, header_set) = parse_lira_csv(csv).expect("parse");
        assert_eq!(rows.len(), 1);
        assert!(skipped.iter().any(|s| s.contains("Owner Name")));
        assert!(unexpected.is_empty());
        assert_eq!(rows[0].business_name.as_deref(), Some("Biz"));
        assert_eq!(header_set, LiraHeaderSet::HtmlTable);
    }

    #[test]
    fn application_date_becomes_iso() {
        let csv = format!(
            "{HEADER}{}",
            row("T", "Biz", "1 St", TYPE_APPLICATION, "9/10/2026")
        );
        let (rows, _, _, header_set) = parse_lira_csv(&csv).expect("parse");
        assert_eq!(rows[0].application_date.as_deref(), Some("2026-09-10"));
        assert_eq!(header_set, LiraHeaderSet::HtmlTable);
    }

    #[test]
    fn ssrs_headers_map_and_attachments_stay_null() {
        let csv = "PROFESSION_FULL_NAME,LICENSEE,LICENSEE_ADDR,TYPE,APPLICATION_DATE\nFood Dealer - Restaurant,Belli's Bistro LLC,\"2230 N MARTIN L KING JR DR Milwaukee, WI 53212\",APPLICATION,9/10/2026\n";
        let (rows, unexpected, skipped, header_set) = parse_lira_csv(csv).expect("parse");
        assert!(unexpected.is_empty());
        assert!(skipped.is_empty());
        assert_eq!(header_set, LiraHeaderSet::Ssrs);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].license_title.as_deref(),
            Some("Food Dealer - Restaurant")
        );
        assert_eq!(rows[0].business_name.as_deref(), Some("Belli's Bistro LLC"));
        assert_eq!(
            rows[0].address.as_deref(),
            Some("2230 N MARTIN L KING JR DR Milwaukee, WI 53212")
        );
        assert_eq!(
            rows[0].license_type_column.as_deref(),
            Some(TYPE_APPLICATION)
        );
        assert_eq!(rows[0].application_date.as_deref(), Some("2026-09-10"));
        assert_eq!(rows[0].has_attachment, None);
    }

    #[test]
    fn mixed_html_and_ssrs_headers_are_labelled_mixed() {
        let csv = "License Title,LICENSEE,Address,TYPE,Application Date\nFood,Biz,1 St,APPLICATION,9/10/2026\n";
        let (rows, unexpected, _, header_set) = parse_lira_csv(csv).expect("parse");
        assert!(unexpected.is_empty());
        assert_eq!(header_set, LiraHeaderSet::Mixed);
        assert_eq!(rows[0].license_title.as_deref(), Some("Food"));
        assert_eq!(rows[0].business_name.as_deref(), Some("Biz"));
        assert_eq!(
            rows[0].license_type_column.as_deref(),
            Some(TYPE_APPLICATION)
        );
    }

    #[test]
    fn null_type_rows_are_excluded_from_the_signal_view() {
        let mut conn = open();
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (99, 'src-milwaukee-lira-applications', 'success', '2026-09-11')",
            [],
        )
        .expect("run");
        conn.execute(
            "INSERT INTO lira_applications (
                 source_file, row_index, load_run_id, fetched_at,
                 license_title, business_name, address, license_type_column,
                 application_date, has_attachment
             ) VALUES ('dead.csv', 0, 99, '2026-09-11T00:00:00Z', NULL, NULL, NULL, NULL, NULL, NULL)",
            [],
        )
        .expect("dead");
        let dir = temp_dir();
        write_csv(
            &dir,
            "live.csv",
            &format!(
                "{HEADER}{}",
                row("T", "Biz", "1 St", TYPE_APPLICATION, "09/10/2026")
            ),
        );
        load_lira_from_folder(&mut conn, &dir).expect("load");
        let signal: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM lira_non_renewal_applications",
                [],
                |r| r.get(0),
            )
            .expect("view");
        assert_eq!(signal, 1);
        let stored: i64 = conn
            .query_row("SELECT COUNT(*) FROM lira_applications", [], |r| r.get(0))
            .expect("all");
        assert_eq!(stored, 2);
        let _ = fs::remove_dir_all(&dir);
    }
}
