// Daily PASDL files are not fetched by HTTP. A constructed GET of
// data.uspto.gov/bulkdata/datasets/pasdl/ad{YYYYMMDD}.zip returns HTTP
// 200 with a 20,666-byte Angular shell, which looks like success and is
// not. api.uspto.gov returns 401 without an ODP API key. The loader
// therefore reads zip files a human already downloaded through a
// browser session, the same pattern as a purchased file.
//
// correspondent is the filing attorney. It is stored for provenance
// only. It is never the business. patent-assignee is the operating
// business.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use radar_config::AssignmentsFile;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use zip::ZipArchive;

use crate::error::StoreError;
use crate::normalise::normalise_owner_name;

pub const USPTO_ASSIGNMENTS_SOURCE_ID: &str = "src-uspto-assignments";
pub const USPTO_DOWNLOAD_PAGE: &str = "https://data.uspto.gov/bulkdata/datasets/pasdl";
pub const USPTO_ASSIGNMENTS_FOLDER: &str =
    r"C:\Users\zumah\Desktop\radar_sources\uspto_assignments";

#[derive(Debug, Clone)]
pub struct FileLoadStats {
    pub source_file: String,
    pub skipped: bool,
    pub empty_file: bool,
    pub parsed: i64,
    pub stored: i64,
    pub discarded: i64,
    pub two_county_assignees: i64,
}

#[derive(Debug, Clone)]
pub struct NameJoinMatch {
    pub assignee_name: String,
    pub parcel_count: i64,
    pub improvement_value_zero_count: i64,
}

#[derive(Debug, Clone)]
pub struct AssignmentLoadResult {
    pub load_run_id: Option<i64>,
    pub empty_folder: bool,
    pub folder: PathBuf,
    pub files: Vec<FileLoadStats>,
    pub assertion_report: String,
    pub profile_report: String,
    pub name_join_report: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentFolderStatus {
    pub folder: String,
    pub download_page: String,
    pub empty: bool,
    pub message: String,
    pub zips_in_folder: i64,
    pub already_loaded: i64,
    pub new_files: i64,
}

pub fn assignment_folder_status(
    conn: &Connection,
    folder: &Path,
) -> Result<AssignmentFolderStatus, StoreError> {
    let zips = list_assignment_zips(folder)?;
    let mut already_loaded = 0i64;
    for zip_path in &zips {
        let source_file = zip_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.zip");
        if source_file_already_loaded(conn, source_file)? {
            already_loaded += 1;
        }
    }
    let zips_in_folder = zips.len() as i64;
    let new_files = zips_in_folder - already_loaded;
    let empty = zips_in_folder == 0;
    let message = if empty {
        format!(
            "assignment folder is empty: {}. Download daily adYYYYMMDD.zip files from {} and place them in that folder.",
            folder.display(),
            USPTO_DOWNLOAD_PAGE
        )
    } else {
        format!(
            "{} zip file(s) in {}. {} already loaded. {} new.",
            zips_in_folder,
            folder.display(),
            already_loaded,
            new_files
        )
    };
    Ok(AssignmentFolderStatus {
        folder: folder.display().to_string(),
        download_page: USPTO_DOWNLOAD_PAGE.to_string(),
        empty,
        message,
        zips_in_folder,
        already_loaded,
        new_files,
    })
}

struct AssigneeRow {
    name: String,
    name_type: Option<String>,
    address_1: Option<String>,
    address_2: Option<String>,
    city: Option<String>,
    state: Option<String>,
    country_name: Option<String>,
    postcode: Option<String>,
}

struct AssignorRow {
    name: Option<String>,
    execution_date: Option<String>,
}

struct PropertyRow {
    country: Option<String>,
    doc_number: Option<String>,
    kind: Option<String>,
    invention_title: Option<String>,
}

struct ParsedAssignment {
    reel_no: String,
    frame_no: String,
    recorded_date: Option<String>,
    last_update_date: Option<String>,
    purge_indicator: Option<String>,
    page_count: Option<i64>,
    conveyance_text: Option<String>,
    correspondent_name: Option<String>,
    correspondent_address: Option<String>,
    assignees: Vec<AssigneeRow>,
    assignors: Vec<AssignorRow>,
    properties: Vec<PropertyRow>,
}

#[derive(Default)]
struct Building {
    reel_no: String,
    frame_no: String,
    recorded_raw: String,
    last_update_raw: String,
    purge_indicator: String,
    page_count_raw: String,
    conveyance_text: String,
    correspondent_name: String,
    correspondent_addr: Vec<String>,
    assignees: Vec<AssigneeRow>,
    assignors: Vec<AssignorRow>,
    properties: Vec<PropertyRow>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    None,
    Record,
    Correspondent,
    Assignor,
    Assignee,
    Property,
    DocId,
    InventionTitle,
}

pub fn prepare_assignments_folder(folder: &Path, downloads: &Path) -> Result<Vec<String>, StoreError> {
    fs::create_dir_all(folder).map_err(|err| {
        StoreError::AssignmentLoad(format!("create folder {}: {err}", folder.display()))
    })?;
    let mut moved = Vec::new();
    if downloads.is_dir() {
        let entries = fs::read_dir(downloads).map_err(|err| {
            StoreError::AssignmentLoad(format!("read downloads {}: {err}", downloads.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|err| StoreError::AssignmentLoad(err.to_string()))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.to_ascii_lowercase().starts_with("ad") || !name.to_ascii_lowercase().ends_with(".zip")
            {
                continue;
            }
            let dest = folder.join(entry.file_name());
            if dest.exists() {
                println!(
                    "already in folder, not moved: {} -> {}",
                    entry.path().display(),
                    dest.display()
                );
                continue;
            }
            fs::rename(entry.path(), &dest).map_err(|err| {
                StoreError::AssignmentLoad(format!(
                    "move {} to {}: {err}",
                    entry.path().display(),
                    dest.display()
                ))
            })?;
            println!("moved {} -> {}", name, dest.display());
            moved.push(name.into_owned());
        }
    }
    Ok(moved)
}

pub fn list_assignment_zips(folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let mut zips = Vec::new();
    if !folder.is_dir() {
        return Ok(zips);
    }
    let entries = fs::read_dir(folder).map_err(|err| {
        StoreError::AssignmentLoad(format!("read folder {}: {err}", folder.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| StoreError::AssignmentLoad(err.to_string()))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_daily_zip_name(name) {
            zips.push(path);
        }
    }
    zips.sort();
    Ok(zips)
}

fn is_daily_zip_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if !lower.starts_with("ad") || !lower.ends_with(".zip") {
        return false;
    }
    let mid = &lower[2..lower.len() - 4];
    mid.len() == 8 && mid.chars().all(|ch| ch.is_ascii_digit())
}

pub fn load_assignments_from_folder(
    conn: &mut Connection,
    folder: &Path,
    config: &AssignmentsFile,
) -> Result<AssignmentLoadResult, StoreError> {
    let zips = list_assignment_zips(folder)?;
    if zips.is_empty() {
        let message = format!(
            "assignment folder is empty: {}. Download daily adYYYYMMDD.zip files from {} and place them in that folder.",
            folder.display(),
            USPTO_DOWNLOAD_PAGE
        );
        println!("{message}");
        return Ok(AssignmentLoadResult {
            load_run_id: None,
            empty_folder: true,
            folder: folder.to_path_buf(),
            files: Vec::new(),
            assertion_report: String::new(),
            profile_report: String::new(),
            name_join_report: String::new(),
        });
    }

    let fetched_at = now_rfc3339();
    let mut file_stats = Vec::new();
    let mut to_insert: Vec<(String, ParsedAssignment)> = Vec::new();
    let mut run_keys: HashSet<(String, String)> = HashSet::new();

    for zip_path in &zips {
        let source_file = zip_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.zip")
            .to_string();
        if source_file_already_loaded(conn, &source_file)? {
            println!("file={source_file} skipped=true already loaded by source_file");
            file_stats.push(FileLoadStats {
                source_file,
                skipped: true,
                empty_file: false,
                parsed: 0,
                stored: 0,
                discarded: 0,
                two_county_assignees: 0,
            });
            continue;
        }

        let parsed = match unzip_and_parse(zip_path) {
            Ok(parsed) => parsed,
            Err(err) => {
                println!("file={source_file} parse failure: {err}");
                return Err(err);
            }
        };
        if parsed.is_empty() {
            println!(
                "file={source_file} no records in this file, which is expected on a day with no recordings"
            );
            file_stats.push(FileLoadStats {
                source_file,
                skipped: false,
                empty_file: true,
                parsed: 0,
                stored: 0,
                discarded: 0,
                two_county_assignees: 0,
            });
            continue;
        }
        let mut stored = 0i64;
        let mut discarded = 0i64;
        let mut two_county = 0i64;
        for rec in parsed {
            if rec.assignees.iter().any(|a| {
                a.state
                    .as_deref()
                    .map(|s| config.is_wisconsin_state(s))
                    .unwrap_or(false)
            }) {
                two_county += rec
                    .assignees
                    .iter()
                    .filter(|a| {
                        a.state
                            .as_deref()
                            .map(|s| config.is_wisconsin_state(s))
                            .unwrap_or(false)
                            && config.derive_county(a.city.as_deref().unwrap_or("")).is_some()
                    })
                    .count() as i64;
                let key = (rec.reel_no.clone(), rec.frame_no.clone());
                if !run_keys.insert(key.clone()) {
                    let message = format!(
                        "duplicate reel_no={} frame_no={} in this load run",
                        key.0, key.1
                    );
                    println!("{message}");
                    return Err(StoreError::AssignmentAssertion {
                        code: "duplicate_reel_frame".to_string(),
                        message,
                    });
                }
                to_insert.push((source_file.clone(), rec));
                stored += 1;
            } else {
                discarded += 1;
            }
        }
        let parsed_count = stored + discarded;
        println!(
            "file={source_file} parsed={parsed_count} wisconsin_stored={stored} discarded={discarded} two_county_assignees={two_county} stored_plus_discarded={parsed_count}"
        );
        file_stats.push(FileLoadStats {
            source_file,
            skipped: false,
            empty_file: false,
            parsed: parsed_count,
            stored,
            discarded,
            two_county_assignees: two_county,
        });
    }

    assert_batch(&to_insert, &file_stats)?;

    let started_at = fetched_at.clone();
    let finished_at = now_rfc3339();
    let written = to_insert.len() as i64;
    let skipped_files = file_stats.iter().filter(|f| f.skipped).count() as i64;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO load_runs (
             source_id, started_at, finished_at, status,
             records_requested, records_written, records_skipped
         ) VALUES (?1, ?2, ?3, 'success', ?4, ?5, ?6)",
        params![
            USPTO_ASSIGNMENTS_SOURCE_ID,
            started_at,
            finished_at,
            file_stats.iter().map(|f| f.parsed).sum::<i64>(),
            written,
            skipped_files
        ],
    )?;
    let load_run_id = tx.last_insert_rowid();
    insert_assignments(&tx, load_run_id, &fetched_at, &to_insert)?;
    tx.commit()?;

    let assertion_report = format_assertions(&file_stats, written);
    println!("{assertion_report}");
    let profile_report = format_profile(conn, config, &file_stats)?;
    println!("{profile_report}");
    let name_join_report = format_name_join(conn, config)?;
    println!("{name_join_report}");

    Ok(AssignmentLoadResult {
        load_run_id: Some(load_run_id),
        empty_folder: false,
        folder: folder.to_path_buf(),
        files: file_stats,
        assertion_report,
        profile_report,
        name_join_report,
    })
}

fn source_file_already_loaded(conn: &Connection, source_file: &str) -> Result<bool, StoreError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM assignment_records WHERE source_file = ?1 LIMIT 1",
            [source_file],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn unzip_and_parse(zip_path: &Path) -> Result<Vec<ParsedAssignment>, StoreError> {
    let tmp = unique_temp_dir("radar_uspto")?;
    let _guard = DeleteDir(tmp.clone());
    let file = File::open(zip_path).map_err(|err| {
        StoreError::AssignmentLoad(format!("open {}: {err}", zip_path.display()))
    })?;
    let mut archive = ZipArchive::new(file).map_err(|err| {
        StoreError::AssignmentLoad(format!("zip {}: {err}", zip_path.display()))
    })?;
    let mut xml_path: Option<PathBuf> = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|err| {
            StoreError::AssignmentLoad(format!("zip entry {}: {err}", zip_path.display()))
        })?;
        let name = entry.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        let dest = tmp.join(
            Path::new(&name)
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("entry")),
        );
        {
            let mut out = File::create(&dest).map_err(|err| {
                StoreError::AssignmentLoad(format!("write unzipped {}: {err}", dest.display()))
            })?;
            std::io::copy(&mut entry, &mut out).map_err(|err| {
                StoreError::AssignmentLoad(format!("extract {}: {err}", dest.display()))
            })?;
        }
        if name.to_ascii_lowercase().ends_with(".xml") {
            xml_path = Some(dest);
        }
    }
    let xml_path = xml_path.ok_or_else(|| {
        StoreError::AssignmentLoad(format!("no xml in {}", zip_path.display()))
    })?;
    let xml = read_xml_lossy(&xml_path)?;
    parse_patent_assignments_xml(&xml)
}

fn read_xml_lossy(path: &Path) -> Result<String, StoreError> {
    let bytes = fs::read(path).map_err(|err| {
        StoreError::AssignmentLoad(format!("read xml {}: {err}", path.display()))
    })?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(err) => Ok(err.into_bytes().iter().map(|&b| b as char).collect()),
    }
}

struct DeleteDir(PathBuf);

impl Drop for DeleteDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, StoreError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| StoreError::AssignmentLoad(err.to_string()))?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}_{}_{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|err| StoreError::AssignmentLoad(err.to_string()))?;
    Ok(dir)
}

fn parse_patent_assignments_xml(xml: &str) -> Result<Vec<ParsedAssignment>, StoreError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut ctx = Ctx::None;
    let mut current: Option<Building> = None;
    let mut out = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if name == "patent-assignment" {
                    current = Some(Building::default());
                    ctx = Ctx::None;
                } else if current.is_some() {
                    match name.as_str() {
                        "assignment-record" => ctx = Ctx::Record,
                        "correspondent" => ctx = Ctx::Correspondent,
                        "patent-assignor" => {
                            if let Some(rec) = current.as_mut() {
                                rec.assignors.push(AssignorRow {
                                    name: None,
                                    execution_date: None,
                                });
                            }
                            ctx = Ctx::Assignor;
                        }
                        "patent-assignee" => {
                            if let Some(rec) = current.as_mut() {
                                rec.assignees.push(AssigneeRow {
                                    name: String::new(),
                                    name_type: None,
                                    address_1: None,
                                    address_2: None,
                                    city: None,
                                    state: None,
                                    country_name: None,
                                    postcode: None,
                                });
                            }
                            ctx = Ctx::Assignee;
                            if let Some(attr) = e.try_get_attribute("name-type").ok().flatten() {
                                if let Some(last) = current.as_mut().and_then(|r| r.assignees.last_mut()) {
                                    last.name_type = Some(
                                        String::from_utf8_lossy(&attr.value).trim().to_string(),
                                    );
                                }
                            }
                        }
                        "patent-property" => {
                            if let Some(rec) = current.as_mut() {
                                rec.properties.push(PropertyRow {
                                    country: None,
                                    doc_number: None,
                                    kind: None,
                                    invention_title: None,
                                });
                            }
                            ctx = Ctx::Property;
                        }
                        "document-id" => ctx = Ctx::DocId,
                        "invention-title" => ctx = Ctx::InventionTitle,
                        "name" => {
                            if ctx == Ctx::Assignee {
                                if let Some(attr) = e.try_get_attribute("name-type").ok().flatten() {
                                    if let Some(last) =
                                        current.as_mut().and_then(|r| r.assignees.last_mut())
                                    {
                                        let value = String::from_utf8_lossy(&attr.value)
                                            .trim()
                                            .to_string();
                                        if !value.is_empty() {
                                            last.name_type = Some(value);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                stack.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                if name == "name" && ctx == Ctx::Assignee {
                    if let Some(attr) = e.try_get_attribute("name-type").ok().flatten() {
                        if let Some(last) = current.as_mut().and_then(|r| r.assignees.last_mut()) {
                            let value = String::from_utf8_lossy(&attr.value).trim().to_string();
                            if !value.is_empty() {
                                last.name_type = Some(value);
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                let text = t
                    .unescape()
                    .map_err(|err| StoreError::AssignmentLoad(format!("xml text: {err}")))?
                    .into_owned();
                if let Some(rec) = current.as_mut() {
                    apply_text(rec, &stack, ctx, &text);
                }
            }
            Ok(Event::CData(t)) => {
                let text = String::from_utf8_lossy(t.as_ref()).into_owned();
                if let Some(rec) = current.as_mut() {
                    apply_text(rec, &stack, ctx, &text);
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                if name == "patent-assignment" {
                    if let Some(built) = current.take() {
                        out.push(finish_building(built)?);
                    }
                    ctx = Ctx::None;
                } else if name == "correspondent" {
                    ctx = Ctx::Record;
                } else if name == "patent-assignor" {
                    ctx = Ctx::None;
                } else if name == "patent-assignee" {
                    ctx = Ctx::None;
                } else if name == "document-id" {
                    ctx = Ctx::Property;
                } else if name == "invention-title" {
                    ctx = Ctx::Property;
                } else if name == "patent-property" {
                    ctx = Ctx::None;
                } else if name == "assignment-record" {
                    ctx = Ctx::None;
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(StoreError::AssignmentLoad(format!("xml parse: {err}")));
            }
            _ => {}
        }
        buf.clear();
    }
    if out.is_empty() && !is_uspto_empty_feed(xml) {
        return Err(StoreError::AssignmentLoad(
            "xml did not contain patent-assignment records and is not a USPTO empty feed".to_string(),
        ));
    }
    Ok(out)
}

fn is_uspto_empty_feed(xml: &str) -> bool {
    let lower = xml.to_ascii_lowercase();
    lower.contains("data-available-code")
        && (lower.contains("<us-patent-assignments") || lower.contains("<patent-assignments"))
}

fn local_name(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    match raw.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => raw.into_owned(),
    }
}

fn apply_text(rec: &mut Building, stack: &[String], ctx: Ctx, text: &str) {
    let tag = stack.last().map(|s| s.as_str()).unwrap_or("");
    let parent = if stack.len() >= 2 {
        stack[stack.len() - 2].as_str()
    } else {
        ""
    };
    match (ctx, tag) {
        (Ctx::Record, "reel-no") => rec.reel_no.push_str(text),
        (Ctx::Record, "frame-no") => rec.frame_no.push_str(text),
        (Ctx::Record, "date") if parent == "recorded-date" => rec.recorded_raw.push_str(text),
        (Ctx::Record, "date") if parent == "last-update-date" => rec.last_update_raw.push_str(text),
        (Ctx::Record, "purge-indicator") => rec.purge_indicator.push_str(text),
        (Ctx::Record, "page-count") => rec.page_count_raw.push_str(text),
        (Ctx::Record, "conveyance-text") => rec.conveyance_text.push_str(text),
        (Ctx::Correspondent, "name") => rec.correspondent_name.push_str(text),
        (Ctx::Correspondent, tag) if tag.starts_with("address-") => {
            rec.correspondent_addr.push(text.to_string());
        }
        (Ctx::Assignor, "name") => {
            if let Some(last) = rec.assignors.last_mut() {
                last.name = Some(append_opt(last.name.as_deref(), text));
            }
        }
        (Ctx::Assignor, "date") if parent == "execution-date" => {
            if let Some(last) = rec.assignors.last_mut() {
                last.execution_date = Some(text.to_string());
            }
        }
        (Ctx::Assignee, "name") => {
            if let Some(last) = rec.assignees.last_mut() {
                last.name.push_str(text);
            }
        }
        (Ctx::Assignee, "address-1") => {
            if let Some(last) = rec.assignees.last_mut() {
                last.address_1 = Some(text.to_string());
            }
        }
        (Ctx::Assignee, "address-2") => {
            if let Some(last) = rec.assignees.last_mut() {
                last.address_2 = Some(text.to_string());
            }
        }
        (Ctx::Assignee, "city") => {
            if let Some(last) = rec.assignees.last_mut() {
                last.city = Some(text.to_string());
            }
        }
        (Ctx::Assignee, "state") => {
            if let Some(last) = rec.assignees.last_mut() {
                last.state = Some(text.to_string());
            }
        }
        (Ctx::Assignee, "country-name") => {
            if let Some(last) = rec.assignees.last_mut() {
                last.country_name = Some(text.to_string());
            }
        }
        (Ctx::Assignee, "postcode") => {
            if let Some(last) = rec.assignees.last_mut() {
                last.postcode = Some(text.to_string());
            }
        }
        (Ctx::DocId, "country") => {
            if let Some(last) = rec.properties.last_mut() {
                last.country = Some(text.to_string());
            }
        }
        (Ctx::DocId, "doc-number") => {
            if let Some(last) = rec.properties.last_mut() {
                last.doc_number = Some(text.to_string());
            }
        }
        (Ctx::DocId, "kind") => {
            if let Some(last) = rec.properties.last_mut() {
                last.kind = Some(text.to_string());
            }
        }
        (Ctx::InventionTitle, _) => {
            if let Some(last) = rec.properties.last_mut() {
                last.invention_title = Some(append_opt(last.invention_title.as_deref(), text));
            }
        }
        _ => {}
    }
}

fn append_opt(existing: Option<&str>, text: &str) -> String {
    match existing {
        Some(prev) if !prev.is_empty() => format!("{prev}{text}"),
        _ => text.to_string(),
    }
}

fn finish_building(built: Building) -> Result<ParsedAssignment, StoreError> {
    let recorded_date = parse_uspto_date(&built.recorded_raw)?;
    let last_update_date = parse_uspto_date(&built.last_update_raw)?;
    let page_count = parse_page_count(&built.page_count_raw)?;
    let correspondent_address = {
        let joined = built
            .correspondent_addr
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    };
    let mut assignors = built.assignors;
    for assignor in &mut assignors {
        assignor.execution_date = match assignor.execution_date.take() {
            Some(raw) => parse_uspto_date(&raw)?,
            None => None,
        };
    }
    Ok(ParsedAssignment {
        reel_no: built.reel_no.trim().to_string(),
        frame_no: built.frame_no.trim().to_string(),
        recorded_date,
        last_update_date,
        purge_indicator: empty_to_none(&built.purge_indicator),
        page_count,
        conveyance_text: empty_to_none(&built.conveyance_text),
        correspondent_name: empty_to_none(&built.correspondent_name),
        correspondent_address,
        assignees: built.assignees,
        assignors,
        properties: built.properties,
    })
}

fn empty_to_none(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_page_count(raw: &str) -> Result<Option<i64>, StoreError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed.parse::<i64>().map(Some).map_err(|_| StoreError::AssignmentAssertion {
        code: "page_count".to_string(),
        message: format!("page-count is not an integer: {trimmed}"),
    })
}

fn parse_uspto_date(raw: &str) -> Result<Option<String>, StoreError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() != 8 || !trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(StoreError::AssignmentAssertion {
            code: "date_parse".to_string(),
            message: format!("date does not parse and was not coerced: {trimmed}"),
        });
    }
    let formatted = format!("{}-{}-{}", &trimmed[0..4], &trimmed[4..6], &trimmed[6..8]);
    if !is_valid_iso_date(&formatted) {
        return Err(StoreError::AssignmentAssertion {
            code: "date_parse".to_string(),
            message: format!("date does not parse and was not coerced: {trimmed}"),
        });
    }
    Ok(Some(formatted))
}

fn is_valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let Ok(year) = value[0..4].parse::<i32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    if month < 1 || month > 12 || day < 1 {
        return false;
    }
    day <= days_in_month(year, month)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn assert_batch(
    rows: &[(String, ParsedAssignment)],
    stats: &[FileLoadStats],
) -> Result<(), StoreError> {
    for stat in stats {
        if !stat.skipped && stat.stored + stat.discarded != stat.parsed {
            return Err(StoreError::AssignmentAssertion {
                code: "parsed_identity".to_string(),
                message: format!(
                    "file={} stored={} discarded={} parsed={}",
                    stat.source_file, stat.stored, stat.discarded, stat.parsed
                ),
            });
        }
    }
    for (_, rec) in rows {
        if rec.reel_no.is_empty() || rec.frame_no.is_empty() {
            return Err(StoreError::AssignmentAssertion {
                code: "reel_frame".to_string(),
                message: "stored assignment missing reel_no or frame_no".to_string(),
            });
        }
        if rec.assignees.is_empty() {
            return Err(StoreError::AssignmentAssertion {
                code: "assignee_required".to_string(),
                message: format!(
                    "stored assignment reel_no={} frame_no={} has no assignee",
                    rec.reel_no, rec.frame_no
                ),
            });
        }
        for assignee in &rec.assignees {
            if assignee.name.trim().is_empty() {
                return Err(StoreError::AssignmentAssertion {
                    code: "assignee_name".to_string(),
                    message: format!(
                        "null or empty assignee name reel_no={} frame_no={}",
                        rec.reel_no, rec.frame_no
                    ),
                });
            }
        }
    }
    Ok(())
}

fn insert_assignments(
    tx: &Transaction<'_>,
    load_run_id: i64,
    fetched_at: &str,
    rows: &[(String, ParsedAssignment)],
) -> Result<(), StoreError> {
    for (source_file, rec) in rows {
        tx.execute(
            "INSERT INTO assignment_records (
                 reel_no, frame_no, recorded_date, last_update_date, purge_indicator,
                 page_count, conveyance_text, correspondent_name, correspondent_address,
                 source_file, load_run_id, fetched_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                rec.reel_no,
                rec.frame_no,
                rec.recorded_date,
                rec.last_update_date,
                rec.purge_indicator,
                rec.page_count,
                rec.conveyance_text,
                rec.correspondent_name,
                rec.correspondent_address,
                source_file,
                load_run_id,
                fetched_at
            ],
        )?;
        for (idx, assignee) in rec.assignees.iter().enumerate() {
            tx.execute(
                "INSERT INTO assignment_assignees (
                     reel_no, frame_no, sequence, name, address_1, address_2, city, state,
                     country_name, postcode, name_type
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    rec.reel_no,
                    rec.frame_no,
                    (idx as i64) + 1,
                    assignee.name.trim(),
                    assignee.address_1.as_deref().map(str::trim),
                    assignee.address_2.as_deref().map(str::trim),
                    assignee.city.as_deref().map(str::trim),
                    assignee.state.as_deref().map(str::trim),
                    assignee.country_name.as_deref().map(str::trim),
                    assignee.postcode.as_deref().map(str::trim),
                    assignee.name_type.as_deref().map(str::trim)
                ],
            )?;
        }
        for (idx, assignor) in rec.assignors.iter().enumerate() {
            tx.execute(
                "INSERT INTO assignment_assignors (
                     reel_no, frame_no, sequence, name, execution_date
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    rec.reel_no,
                    rec.frame_no,
                    (idx as i64) + 1,
                    assignor.name.as_deref().map(str::trim),
                    assignor.execution_date
                ],
            )?;
        }
        for (idx, prop) in rec.properties.iter().enumerate() {
            tx.execute(
                "INSERT INTO assignment_properties (
                     reel_no, frame_no, sequence, country, doc_number, kind, invention_title
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    rec.reel_no,
                    rec.frame_no,
                    (idx as i64) + 1,
                    prop.country.as_deref().map(str::trim),
                    prop.doc_number.as_deref().map(str::trim),
                    prop.kind.as_deref().map(str::trim),
                    prop.invention_title.as_deref().map(str::trim)
                ],
            )?;
        }
    }
    Ok(())
}

fn format_assertions(stats: &[FileLoadStats], written: i64) -> String {
    let mut lines = vec!["USPTO assignment assertions".to_string()];
    for stat in stats {
        if stat.skipped {
            lines.push(format!("file={} skipped=true", stat.source_file));
        } else if stat.empty_file {
            lines.push(format!(
                "file={} no records in this file, which is expected on a day with no recordings",
                stat.source_file
            ));
        } else {
            lines.push(format!(
                "file={} parsed={} stored={} discarded={} stored_plus_discarded={}",
                stat.source_file,
                stat.parsed,
                stat.stored,
                stat.discarded,
                stat.stored + stat.discarded
            ));
        }
    }
    lines.push(format!("inserted assignment records={written}"));
    lines.join("\n")
}

fn format_profile(
    conn: &Connection,
    config: &AssignmentsFile,
    stats: &[FileLoadStats],
) -> Result<String, StoreError> {
    let mut lines = vec!["USPTO assignment profile".to_string()];
    for stat in stats {
        if stat.skipped {
            lines.push(format!("file={} skipped=true", stat.source_file));
        } else if stat.empty_file {
            lines.push(format!(
                "file={} no records in this file, which is expected on a day with no recordings",
                stat.source_file
            ));
        } else {
            lines.push(format!(
                "file={} records_parsed={} wisconsin_stored={} two_county_assignees={}",
                stat.source_file, stat.parsed, stat.stored, stat.two_county_assignees
            ));
        }
    }

    let total_records: i64 = conn.query_row("SELECT COUNT(*) FROM assignment_records", [], |row| row.get(0))?;
    let total_assignees: i64 =
        conn.query_row("SELECT COUNT(*) FROM assignment_assignees", [], |row| row.get(0))?;
    lines.push(format!("total records={total_records}"));
    lines.push(format!("total assignees={total_assignees}"));

    let mut waukesha = 0i64;
    let mut milwaukee = 0i64;
    let mut unmatched: BTreeMap<String, i64> = BTreeMap::new();
    #[derive(Default)]
    struct NameAgg {
        records: HashSet<(String, String)>,
        city: String,
        recorded: Option<String>,
    }
    let mut by_name: HashMap<String, NameAgg> = HashMap::new();
    let mut two_county_names: HashSet<String> = HashSet::new();

    {
        let mut key_stmt = conn.prepare(
            "SELECT a.reel_no, a.frame_no, a.name, a.city, a.state, r.recorded_date
             FROM assignment_assignees a
             JOIN assignment_records r ON r.reel_no = a.reel_no AND r.frame_no = a.frame_no",
        )?;
        let key_rows = key_stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        for row in key_rows {
            let (reel, frame, name, city, state, recorded) = row?;
            let state_val = state.as_deref().unwrap_or("");
            let city_val = city.as_deref().unwrap_or("");
            if config.is_wisconsin_state(state_val)
                && !city_val.is_empty()
                && config.derive_county(city_val).is_none()
            {
                let key = city_val.to_string();
                *unmatched.entry(key).or_insert(0) += 1;
            }
            if config.is_wisconsin_state(state_val) {
                if let Some(county) = config.derive_county(city_val) {
                    if county == "waukesha" {
                        waukesha += 1;
                    } else {
                        milwaukee += 1;
                    }
                    two_county_names.insert(name.clone());
                    let agg = by_name.entry(name.clone()).or_default();
                    agg.records.insert((reel, frame));
                    let newer = match (&agg.recorded, &recorded) {
                        (None, Some(_)) => true,
                        (Some(prev), Some(next)) => next > prev,
                        _ => false,
                    };
                    if newer || agg.city.is_empty() {
                        agg.city = city_val.to_string();
                        if recorded.is_some() {
                            agg.recorded = recorded;
                        }
                    }
                }
            }
        }
    }

    lines.push(format!("waukesha county assignees={waukesha}"));
    lines.push(format!("milwaukee county assignees={milwaukee}"));

    lines.push("recorded_date by year:".to_string());
    append_year_counts(conn, "recorded_date", &mut lines)?;
    lines.push("last_update_date by year:".to_string());
    append_year_counts(conn, "last_update_date", &mut lines)?;

    lines.push("conveyance_text counts:".to_string());
    let mut conv = conn.prepare(
        "SELECT COALESCE(conveyance_text, '') AS t, COUNT(*) AS n
         FROM assignment_records GROUP BY t ORDER BY n DESC, t",
    )?;
    let conv_rows = conv.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in conv_rows {
        let (text, n) = row?;
        lines.push(format!("  {n} {text}"));
    }

    lines.push("wisconsin cities matching no list:".to_string());
    if unmatched.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for (city, n) in &unmatched {
            lines.push(format!("  {n} {city}"));
        }
    }

    let mut ranked: Vec<(String, NameAgg)> = by_name.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.records
            .len()
            .cmp(&a.1.records.len())
            .then_with(|| a.0.cmp(&b.0))
    });
    lines.push("top 40 two-county assignee names:".to_string());
    for (idx, (name, agg)) in ranked.iter().take(40).enumerate() {
        lines.push(format!(
            "  {}. count={} city={} most_recent_recorded_date={} name={}",
            idx + 1,
            agg.records.len(),
            agg.city,
            agg.recorded.as_deref().unwrap_or(""),
            name
        ));
    }
    lines.push(format!(
        "distinct two-county company names={}",
        two_county_names.len()
    ));
    Ok(lines.join("\n"))
}

fn append_year_counts(
    conn: &Connection,
    column: &str,
    lines: &mut Vec<String>,
) -> Result<(), StoreError> {
    for (year, n) in year_counts(conn, column)? {
        lines.push(format!("  {year}: {n}"));
    }
    Ok(())
}

fn year_counts(conn: &Connection, column: &str) -> Result<Vec<(String, i64)>, StoreError> {
    let sql = format!(
        "SELECT substr({column}, 1, 4), COUNT(*)
         FROM assignment_records
         WHERE {column} IS NOT NULL AND trim({column}) != ''
         GROUP BY substr({column}, 1, 4)
         ORDER BY 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut counts = Vec::new();
    while let Some(row) = rows.next()? {
        counts.push((row.get::<_, String>(0)?, row.get::<_, i64>(1)?));
    }
    Ok(counts)
}

fn format_name_join(conn: &Connection, config: &AssignmentsFile) -> Result<String, StoreError> {
    let mut names: HashSet<String> = HashSet::new();
    let mut stmt = conn.prepare("SELECT name, city, state FROM assignment_assignees")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (name, city, state) = row?;
        if state
            .as_deref()
            .map(|s| config.is_wisconsin_state(s))
            .unwrap_or(false)
            && config.derive_county(city.as_deref().unwrap_or("")).is_some()
        {
            names.insert(name);
        }
    }

    let mut parcels: HashMap<String, (i64, i64)> = HashMap::new();
    let mut pstmt = conn.prepare(
        "SELECT owner_name, COUNT(*), SUM(CASE WHEN improvement_value = 0 THEN 1 ELSE 0 END)
         FROM current_parcels
         WHERE owner_name IS NOT NULL AND trim(owner_name) != ''
         GROUP BY owner_name",
    )?;
    let prows = pstmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    for row in prows {
        let (owner, parcels_n, zeros) = row?;
        let key = normalise_owner_name(&owner);
        if key.is_empty() {
            continue;
        }
        let entry = parcels.entry(key).or_insert((0, 0));
        entry.0 += parcels_n;
        entry.1 += zeros.unwrap_or(0);
    }

    let mut matches = Vec::new();
    for name in &names {
        let key = normalise_owner_name(name);
        if let Some((parcel_count, zeros)) = parcels.get(&key) {
            matches.push(NameJoinMatch {
                assignee_name: name.clone(),
                parcel_count: *parcel_count,
                improvement_value_zero_count: *zeros,
            });
        }
    }
    matches.sort_by(|a, b| {
        b.parcel_count
            .cmp(&a.parcel_count)
            .then_with(|| a.assignee_name.cmp(&b.assignee_name))
    });

    let denom = names.len();
    let rate = if denom == 0 {
        0.0
    } else {
        matches.len() as f64 / denom as f64
    };
    let mut lines = vec![
        "USPTO two-county name join against current_parcels (report only, no repair)".to_string(),
        format!(
            "matches={} distinct_two_county_names={} rate={rate:.6}",
            matches.len(),
            denom
        ),
    ];
    if matches.is_empty() {
        lines.push("matches: (none)".to_string());
    } else {
        lines.push("matches:".to_string());
        for m in matches {
            lines.push(format!(
                "  name={} parcel_count={} improvement_value_zero_count={}",
                m.assignee_name, m.parcel_count, m.improvement_value_zero_count
            ));
        }
    }
    Ok(lines.join("\n"))
}

fn now_rfc3339() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock");
    let secs = now.as_secs() as i64;
    let nanos = now.subsec_nanos();
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 }.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    let hour = sod / 3600;
    let min = (sod % 3600) / 60;
    let sec = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}.{nanos:09}+00:00")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::migrate;
    use rusqlite::Connection;
    use std::io::Write;
    use zip::write::FileOptions;
    use zip::ZipWriter;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    fn cfg() -> AssignmentsFile {
        AssignmentsFile::from_dr_data_file()
    }

    fn xml_record(reel: &str, frame: &str, city: &str, state: &str, name: &str) -> String {
        format!(
            r#"<patent-assignment>
  <assignment-record>
    <reel-no>{reel}</reel-no>
    <frame-no>{frame}</frame-no>
    <last-update-date><date>20260904</date></last-update-date>
    <purge-indicator>N</purge-indicator>
    <recorded-date><date>20240115</date></recorded-date>
    <page-count>1</page-count>
    <correspondent><name>LAW FIRM LLP</name><address-1>1 MAIN</address-1></correspondent>
    <conveyance-text>ASSIGNMENT OF ASSIGNORS INTEREST</conveyance-text>
  </assignment-record>
  <patent-assignors>
    <patent-assignor><name>INVENTOR</name><execution-date><date>20240101</date></execution-date></patent-assignor>
  </patent-assignors>
  <patent-assignees>
    <patent-assignee>
      <name>{name}</name>
      <address-1>1 ST</address-1>
      <city>{city}</city>
      <state>{state}</state>
      <country-name>UNITED STATES</country-name>
      <postcode>53186</postcode>
    </patent-assignee>
  </patent-assignees>
  <patent-properties>
    <patent-property>
      <document-id><country>US</country><doc-number>1</doc-number><kind>A1</kind></document-id>
      <invention-title>Widget</invention-title>
    </patent-property>
  </patent-properties>
</patent-assignment>"#
        )
    }

    fn wrap(body: &str) -> String {
        format!("<?xml version=\"1.0\"?><patent-assignments>{body}</patent-assignments>")
    }

    fn write_zip(dir: &Path, zip_name: &str, xml: &str) {
        let path = dir.join(zip_name);
        let file = File::create(&path).expect("zip");
        let mut zip = ZipWriter::new(file);
        let xml_name = zip_name.replace(".zip", ".xml");
        zip.start_file(&xml_name, FileOptions::default())
            .expect("start");
        zip.write_all(xml.as_bytes()).expect("write xml");
        zip.finish().expect("finish");
    }

    fn temp_folder() -> PathBuf {
        unique_temp_dir("radar_uspto_test").expect("tmp")
    }

    #[test]
    fn non_wisconsin_record_is_discarded_and_counted() {
        let mut conn = open();
        let dir = temp_folder();
        let xml = wrap(&xml_record("1", "1", "Chicago", "IL", "OUT OF STATE LLC"));
        write_zip(&dir, "ad20260904.zip", &xml);
        let result = load_assignments_from_folder(&mut conn, &dir, &cfg()).expect("load");
        assert_eq!(result.files[0].parsed, 1);
        assert_eq!(result.files[0].stored, 0);
        assert_eq!(result.files[0].discarded, 1);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM assignment_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dirty_city_is_stored_as_unmatched_not_dropped() {
        let mut conn = open();
        let dir = temp_folder();
        let xml = wrap(&xml_record("2", "2", "WAUKESH", "WI", "DIRTY CITY LLC"));
        write_zip(&dir, "ad20260904.zip", &xml);
        load_assignments_from_folder(&mut conn, &dir, &cfg()).expect("load");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM assignment_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let city: String = conn
            .query_row("SELECT city FROM assignment_assignees", [], |r| r.get(0))
            .unwrap();
        assert_eq!(city, "WAUKESH");
        assert!(cfg().derive_county("WAUKESH").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_reel_and_frame_in_one_run_fails() {
        let mut conn = open();
        let dir = temp_folder();
        let body = format!(
            "{}{}",
            xml_record("9", "9", "Waukesha", "WI", "ONE LLC"),
            xml_record("9", "9", "Milwaukee", "WI", "TWO LLC")
        );
        write_zip(&dir, "ad20260904.zip", &wrap(&body));
        let err = load_assignments_from_folder(&mut conn, &dir, &cfg()).unwrap_err();
        match err {
            StoreError::AssignmentAssertion { code, .. } => {
                assert_eq!(code, "duplicate_reel_frame");
            }
            other => panic!("expected duplicate, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn previously_loaded_file_is_skipped() {
        let mut conn = open();
        let dir = temp_folder();
        let xml = wrap(&xml_record("3", "3", "Waukesha", "WI", "SKIP ME LLC"));
        write_zip(&dir, "ad20260904.zip", &xml);
        load_assignments_from_folder(&mut conn, &dir, &cfg()).expect("first");
        let result = load_assignments_from_folder(&mut conn, &dir, &cfg()).expect("second");
        assert!(result.files[0].skipped);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM assignment_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM load_runs WHERE source_id = 'src-uspto-assignments'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runs, 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn county_is_derived_not_stored() {
        let conn = open();
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'assignment_assignees'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !sql.to_ascii_lowercase().contains("county"),
            "county must not be stored: {sql}"
        );
        assert_eq!(cfg().derive_county("waukesha"), Some("waukesha"));
        assert_eq!(cfg().derive_county("MILWAUKEE"), Some("milwaukee"));
    }

    fn insert_record(
        conn: &Connection,
        reel: &str,
        frame: &str,
        recorded: &str,
        last_update: &str,
        city: &str,
        name: &str,
    ) {
        conn.execute(
            "INSERT INTO assignment_records (
                 reel_no, frame_no, recorded_date, last_update_date, conveyance_text,
                 correspondent_name, source_file, load_run_id, fetched_at
             ) VALUES (?1, ?2, ?3, ?4, 'ASSIGNMENT', 'LAW FIRM LLP', 'ad20260904.zip', 1, '2026-09-04T00:00:00+00:00')",
            rusqlite::params![reel, frame, recorded, last_update],
        )
        .expect("record");
        conn.execute(
            "INSERT INTO assignment_assignees (
                 reel_no, frame_no, sequence, name, city, state
             ) VALUES (?1, ?2, 1, ?3, ?4, 'WI')",
            rusqlite::params![reel, frame, name, city],
        )
        .expect("assignee");
    }

    #[test]
    fn recorded_date_by_year_is_not_mixed_with_cities() {
        let conn = open();
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (1, 'src-uspto-assignments', 'success', '2026-09-04')",
            [],
        )
        .expect("run");
        insert_record(&conn, "1", "1", "2021-05-06", "2026-09-04", "Waukesha", "A LLC");
        insert_record(&conn, "1", "2", "2023-06-05", "2026-09-04", "Milwaukee", "B LLC");
        insert_record(&conn, "1", "3", "2024-01-16", "2026-09-04", "Brookfield", "C LLC");
        insert_record(&conn, "1", "4", "2025-12-03", "2026-09-04", "WAUKESH", "D LLC");
        insert_record(&conn, "1", "5", "2026-09-03", "2026-09-04", "Madison", "E LLC");
        let report = super::format_profile(&conn, &cfg(), &[]).expect("profile");
        let year_block = report
            .split("recorded_date by year:")
            .nth(1)
            .unwrap()
            .split("last_update_date by year:")
            .next()
            .unwrap();
        assert!(year_block.contains("2021: 1"), "{year_block}");
        assert!(year_block.contains("2023: 1"), "{year_block}");
        assert!(year_block.contains("2024: 1"), "{year_block}");
        assert!(year_block.contains("2025: 1"), "{year_block}");
        assert!(year_block.contains("2026: 1"), "{year_block}");
        assert!(
            !year_block.to_ascii_lowercase().contains("waukesh"),
            "city leaked into year block: {year_block}"
        );
        assert!(
            !year_block.to_ascii_lowercase().contains("madison"),
            "city leaked into year block: {year_block}"
        );
        assert!(report.contains("wisconsin cities matching no list:"));
        assert!(report.contains("WAUKESH"));
    }

    #[test]
    fn empty_feed_file_is_explained() {
        let mut conn = open();
        let dir = temp_folder();
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<us-patent-assignments>
  <action-key-code>DA</action-key-code>
  <transaction-date><date>20260905</date></transaction-date>
  <patent-assignments>
    <data-available-code>N</data-available-code>
  </patent-assignments>
</us-patent-assignments>"#;
        write_zip(&dir, "ad20260905.zip", xml);
        let result = load_assignments_from_folder(&mut conn, &dir, &cfg()).expect("load");
        assert_eq!(result.files.len(), 1);
        assert!(result.files[0].empty_file);
        assert_eq!(result.files[0].parsed, 0);
        assert!(result.profile_report.contains(
            "no records in this file, which is expected on a day with no recordings"
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_failure_is_an_error_not_parsed_zero() {
        let mut conn = open();
        let dir = temp_folder();
        write_zip(&dir, "ad20260906.zip", "<html><body>Angular shell</body></html>");
        let err = load_assignments_from_folder(&mut conn, &dir, &cfg()).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("parse") || message.contains("xml") || message.contains("empty feed"),
            "{message}"
        );
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM assignment_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
