use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use radar_config::{RadarConfig, SourcesFile};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::assignments::{
    assignment_folder_status, load_assignments_from_folder, AssignmentFolderStatus,
    USPTO_ASSIGNMENTS_SOURCE_ID, USPTO_DOWNLOAD_PAGE,
};
use crate::list::DEFAULT_PAGE_LIMIT;
use crate::normalise::normalise_owner_name;
use crate::outcomes::{assignment_subject_key, latest_outcomes, LatestOutcome, SUBJECT_ASSIGNMENT_ASSIGNEE};
use crate::StoreError;

pub const CORRESPONDENT_ATTORNEY_SENTENCE: &str =
    "correspondent is the filing attorney rather than the business.";

const COUNTY_UNMATCHED: &str = "unmatched";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AssignmentListQuery {
    pub county: Option<String>,
    pub city: Option<String>,
    pub conveyance_text: Option<String>,
    pub recorded_date_from: Option<String>,
    pub recorded_date_to: Option<String>,
    pub last_update_date_from: Option<String>,
    pub last_update_date_to: Option<String>,
    pub name_contains: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssignmentParcelLink {
    pub parcel_count: i64,
    pub improvement_value_zero_count: i64,
    pub group_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentListRow {
    pub reel_no: String,
    pub frame_no: String,
    pub sequence: i64,
    pub company_name: String,
    pub city: Option<String>,
    pub county: Option<String>,
    pub county_unmatched: bool,
    pub address_1: Option<String>,
    pub address_2: Option<String>,
    pub postcode: Option<String>,
    pub conveyance_text: Option<String>,
    pub recorded_date: Option<String>,
    pub last_update_date: Option<String>,
    pub patent_count: i64,
    pub assignor_names: String,
    pub filing_attorney_name: Option<String>,
    pub parcel: Option<AssignmentParcelLink>,
    pub load_run_id: i64,
    pub source_file: String,
    pub latest_outcome: Option<LatestOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentParcelMatchRate {
    pub matched: i64,
    pub distinct_names: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentStaleness {
    pub newest_last_update_date: Option<String>,
    pub days_old: Option<i64>,
    pub cadence_days: Option<u32>,
    pub stale: bool,
    pub download_page: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentListPage {
    pub rows: Vec<AssignmentListRow>,
    pub total_before_filter: i64,
    pub total_after_filter: i64,
    pub offset: i64,
    pub limit: i64,
    pub active_filters_text: String,
    pub parcel_match: AssignmentParcelMatchRate,
    pub staleness: AssignmentStaleness,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentFilterOptions {
    pub counties: Vec<String>,
    pub cities: Vec<String>,
    pub conveyance_texts: Vec<String>,
    pub vertical_display_name: String,
    pub config_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentAssignee {
    pub sequence: i64,
    pub name: String,
    pub address_1: Option<String>,
    pub address_2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postcode: Option<String>,
    pub county: Option<String>,
    pub county_unmatched: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentAssignor {
    pub sequence: i64,
    pub name: Option<String>,
    pub execution_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentProperty {
    pub sequence: i64,
    pub country: Option<String>,
    pub doc_number: Option<String>,
    pub kind: Option<String>,
    pub invention_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentDetail {
    pub reel_no: String,
    pub frame_no: String,
    pub recorded_date: Option<String>,
    pub last_update_date: Option<String>,
    pub conveyance_text: Option<String>,
    pub filing_attorney_name: Option<String>,
    pub filing_attorney_address: Option<String>,
    pub assignees: Vec<AssignmentAssignee>,
    pub assignors: Vec<AssignmentAssignor>,
    pub properties: Vec<AssignmentProperty>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnmatchedCity {
    pub city: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentRefreshResult {
    pub folder_status: AssignmentFolderStatus,
    pub empty_folder: bool,
    pub files_processed: i64,
    pub files_skipped: i64,
    pub records_parsed: i64,
    pub wisconsin_stored: i64,
    pub two_county_assignees: i64,
    pub unmatched_wisconsin_cities: Vec<UnmatchedCity>,
    pub load_run_id: Option<i64>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentExportMeta {
    pub config_directory: String,
    pub config_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentExportResult {
    pub csv_path: String,
    pub provenance_path: String,
    pub rows_exported: i64,
}

pub fn assignment_cadence_days(sources: &SourcesFile) -> Option<u32> {
    sources
        .sources
        .iter()
        .find(|source| source.source_id == USPTO_ASSIGNMENTS_SOURCE_ID)
        .and_then(|source| source.cadence_days)
}

pub fn assignment_download_page(sources: &SourcesFile) -> String {
    sources
        .sources
        .iter()
        .find(|source| source.source_id == USPTO_ASSIGNMENTS_SOURCE_ID)
        .and_then(|source| source.url.clone())
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| USPTO_DOWNLOAD_PAGE.to_string())
}

pub fn assignment_staleness(
    conn: &Connection,
    sources: &SourcesFile,
) -> Result<AssignmentStaleness, StoreError> {
    let cadence_days = assignment_cadence_days(sources);
    let download_page = assignment_download_page(sources);
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT last_update_date,
                    CAST(julianday(date('now')) - julianday(last_update_date) AS INTEGER)
             FROM assignment_records
             WHERE last_update_date IS NOT NULL AND trim(last_update_date) != ''
             ORDER BY last_update_date DESC
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (newest_last_update_date, days_old) = match row {
        Some((date, days)) => (Some(date), Some(days)),
        None => (None, None),
    };
    let stale = match (days_old, cadence_days) {
        (Some(days), Some(cadence)) => days > i64::from(cadence),
        _ => false,
    };
    let text = match (&newest_last_update_date, days_old, cadence_days, stale) {
        (None, _, _, _) => {
            "No last_update_date is stored yet.".to_string()
        }
        (Some(date), Some(days), Some(cadence), true) => format!(
            "Newest last_update_date in the store is {date}, {days} days old. That is more than the {cadence}-day cadence in the source registry. Daily files arrive by hand and nothing else will remind you. Download them from {download_page}."
        ),
        (Some(date), Some(days), Some(cadence), false) => format!(
            "Newest last_update_date in the store is {date}, {days} days old. Source registry cadence is {cadence} days."
        ),
        (Some(date), Some(days), None, _) => format!(
            "Newest last_update_date in the store is {date}, {days} days old. No cadence_days is set for {USPTO_ASSIGNMENTS_SOURCE_ID} in the source registry."
        ),
        (Some(date), None, _, _) => {
            format!("Newest last_update_date in the store is {date}.")
        }
    };
    Ok(AssignmentStaleness {
        newest_last_update_date,
        days_old,
        cadence_days,
        stale,
        download_page,
        text,
    })
}

pub fn assignment_filter_options(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<AssignmentFilterOptions, StoreError> {
    let rows = load_assignee_rows(config, conn)?;
    let mut counties = BTreeSet::new();
    let mut cities = BTreeSet::new();
    let mut conveyances = BTreeSet::new();
    let mut has_unmatched = false;
    for row in &rows {
        if let Some(county) = row.county.as_ref() {
            counties.insert(county.clone());
        }
        if row.county_unmatched {
            has_unmatched = true;
        }
        if let Some(city) = row.city.as_ref() {
            if !city.is_empty() {
                cities.insert(city.clone());
            }
        }
        if let Some(text) = row.conveyance_text.as_ref() {
            if !text.is_empty() {
                conveyances.insert(text.clone());
            }
        }
    }
    let mut counties: Vec<String> = counties.into_iter().collect();
    if has_unmatched {
        counties.push(COUNTY_UNMATCHED.to_string());
    }
    Ok(AssignmentFilterOptions {
        counties,
        cities: cities.into_iter().collect(),
        conveyance_texts: conveyances.into_iter().collect(),
        vertical_display_name: config.manifest.vertical_display_name.clone(),
        config_version: config.manifest.config_version.clone(),
    })
}

pub fn list_assignments(
    config: &RadarConfig,
    conn: &Connection,
    query: &AssignmentListQuery,
) -> Result<AssignmentListPage, StoreError> {
    let cleaned = clean_query(query);
    let mut rows = load_assignee_rows(config, conn)?;
    let total_before_filter = rows.len() as i64;
    rows.retain(|row| row_matches(row, &cleaned));
    let total_after_filter = rows.len() as i64;
    rows.sort_by(compare_rows);
    let offset = cleaned.offset.unwrap_or(0).max(0);
    let limit = match cleaned.limit {
        Some(value) if value > 0 => value,
        _ => DEFAULT_PAGE_LIMIT,
    };
    let page_rows: Vec<AssignmentListRow> = rows
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let keys: Vec<String> = page_rows
        .iter()
        .map(|row| assignment_subject_key(&row.reel_no, &row.frame_no, row.sequence))
        .collect();
    let latest = latest_outcomes(conn, SUBJECT_ASSIGNMENT_ASSIGNEE, &keys)?;
    let page_rows = page_rows
        .into_iter()
        .map(|mut row| {
            let key = assignment_subject_key(&row.reel_no, &row.frame_no, row.sequence);
            row.latest_outcome = latest.get(&key).cloned();
            row
        })
        .collect();
    Ok(AssignmentListPage {
        rows: page_rows,
        total_before_filter,
        total_after_filter,
        offset,
        limit,
        active_filters_text: active_filters_text(&cleaned),
        parcel_match: parcel_match_rate(config, conn)?,
        staleness: assignment_staleness(conn, &config.sources)?,
    })
}

pub fn assignment_detail(
    config: &RadarConfig,
    conn: &Connection,
    reel_no: &str,
    frame_no: &str,
) -> Result<AssignmentDetail, StoreError> {
    let found: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = conn
        .query_row(
            "SELECT recorded_date, last_update_date, conveyance_text,
                    correspondent_name, correspondent_address
             FROM assignment_records
             WHERE reel_no = ?1 AND frame_no = ?2",
            rusqlite::params![reel_no, frame_no],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((recorded_date, last_update_date, conveyance_text, filing_attorney_name, filing_attorney_address)) =
        found
    else {
        return Err(StoreError::AssignmentLoad(format!(
            "assignment reel_no={reel_no} frame_no={frame_no} was not found"
        )));
    };

    let mut assignee_stmt = conn.prepare(
        "SELECT sequence, name, address_1, address_2, city, state, postcode
         FROM assignment_assignees
         WHERE reel_no = ?1 AND frame_no = ?2
         ORDER BY sequence",
    )?;
    let assignee_rows = assignee_stmt.query_map(rusqlite::params![reel_no, frame_no], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut assignees = Vec::new();
    for row in assignee_rows {
        let (sequence, name, address_1, address_2, city, state, postcode) = row?;
        let (county, county_unmatched) = derive_county_flags(&config.assignments, city.as_deref());
        assignees.push(AssignmentAssignee {
            sequence,
            name,
            address_1,
            address_2,
            city,
            state,
            postcode,
            county,
            county_unmatched,
        });
    }

    let mut assignor_stmt = conn.prepare(
        "SELECT sequence, name, execution_date
         FROM assignment_assignors
         WHERE reel_no = ?1 AND frame_no = ?2
         ORDER BY sequence",
    )?;
    let assignor_rows = assignor_stmt.query_map(rusqlite::params![reel_no, frame_no], |row| {
        Ok(AssignmentAssignor {
            sequence: row.get(0)?,
            name: row.get(1)?,
            execution_date: row.get(2)?,
        })
    })?;
    let mut assignors = Vec::new();
    for row in assignor_rows {
        assignors.push(row?);
    }

    let mut prop_stmt = conn.prepare(
        "SELECT sequence, country, doc_number, kind, invention_title
         FROM assignment_properties
         WHERE reel_no = ?1 AND frame_no = ?2
         ORDER BY sequence",
    )?;
    let prop_rows = prop_stmt.query_map(rusqlite::params![reel_no, frame_no], |row| {
        Ok(AssignmentProperty {
            sequence: row.get(0)?,
            country: row.get(1)?,
            doc_number: row.get(2)?,
            kind: row.get(3)?,
            invention_title: row.get(4)?,
        })
    })?;
    let mut properties = Vec::new();
    for row in prop_rows {
        properties.push(row?);
    }

    Ok(AssignmentDetail {
        reel_no: reel_no.to_string(),
        frame_no: frame_no.to_string(),
        recorded_date,
        last_update_date,
        conveyance_text,
        filing_attorney_name,
        filing_attorney_address,
        assignees,
        assignors,
        properties,
    })
}

pub fn refresh_assignments(
    conn: &mut Connection,
    folder: &Path,
    config: &radar_config::AssignmentsFile,
) -> Result<AssignmentRefreshResult, StoreError> {
    let folder_status = assignment_folder_status(conn, folder)?;
    if folder_status.empty {
        return Ok(AssignmentRefreshResult {
            summary: folder_status.message.clone(),
            folder_status,
            empty_folder: true,
            files_processed: 0,
            files_skipped: 0,
            records_parsed: 0,
            wisconsin_stored: 0,
            two_county_assignees: 0,
            unmatched_wisconsin_cities: unmatched_wisconsin_cities(conn, config)?,
            load_run_id: None,
        });
    }
    let loaded = load_assignments_from_folder(conn, folder, config)?;
    let files_processed = loaded.files.len() as i64;
    let files_skipped = loaded.files.iter().filter(|file| file.skipped).count() as i64;
    let records_parsed = loaded.files.iter().map(|file| file.parsed).sum();
    let wisconsin_stored = loaded.files.iter().map(|file| file.stored).sum();
    let two_county_assignees = loaded
        .files
        .iter()
        .map(|file| file.two_county_assignees)
        .sum();
    let unmatched_wisconsin_cities = unmatched_wisconsin_cities(conn, config)?;
    let unmatched_text = if unmatched_wisconsin_cities.is_empty() {
        "Wisconsin cities matching no list: none".to_string()
    } else {
        let listed = unmatched_wisconsin_cities
            .iter()
            .map(|city| format!("{} ({})", city.city, city.count))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Wisconsin cities matching no list: {listed}")
    };
    let summary = format!(
        "files processed={files_processed} (skipped={files_skipped}). records parsed={records_parsed}. Wisconsin stored={wisconsin_stored}. two-county assignees={two_county_assignees}. {unmatched_text}."
    );
    Ok(AssignmentRefreshResult {
        folder_status,
        empty_folder: loaded.empty_folder,
        files_processed,
        files_skipped,
        records_parsed,
        wisconsin_stored,
        two_county_assignees,
        unmatched_wisconsin_cities,
        load_run_id: loaded.load_run_id,
        summary,
    })
}

pub fn export_assignments(
    config: &RadarConfig,
    conn: &Connection,
    query: &AssignmentListQuery,
    path: &Path,
    meta: &AssignmentExportMeta,
) -> Result<AssignmentExportResult, StoreError> {
    let mut export_query = clean_query(query);
    export_query.offset = Some(0);
    export_query.limit = Some(i64::MAX);
    let page = list_assignments(config, conn, &export_query)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                StoreError::AssignmentLoad(format!("create export directory: {err}"))
            })?;
        }
    }
    let mut csv = String::new();
    csv.push_str(
        "company_name,city,county,address_1,address_2,postcode,conveyance_text,recorded_date,last_update_date,reel_no,frame_no,patent_count,assignor_names,filing_attorney_correspondent_name,parcel_count,improvement_value_zero_count,source_file,load_run_id\n",
    );
    let mut load_runs: BTreeSet<i64> = BTreeSet::new();
    let mut source_files: BTreeSet<String> = BTreeSet::new();
    for row in &page.rows {
        load_runs.insert(row.load_run_id);
        source_files.insert(row.source_file.clone());
        let (parcel_count, zeros) = match &row.parcel {
            Some(link) => (
                link.parcel_count.to_string(),
                link.improvement_value_zero_count.to_string(),
            ),
            None => (String::new(), String::new()),
        };
        let county = if row.county_unmatched {
            COUNTY_UNMATCHED.to_string()
        } else {
            row.county.clone().unwrap_or_default()
        };
        csv.push_str(&csv_row(&[
            row.company_name.clone(),
            row.city.clone().unwrap_or_default(),
            county,
            row.address_1.clone().unwrap_or_default(),
            row.address_2.clone().unwrap_or_default(),
            row.postcode.clone().unwrap_or_default(),
            row.conveyance_text.clone().unwrap_or_default(),
            row.recorded_date.clone().unwrap_or_default(),
            row.last_update_date.clone().unwrap_or_default(),
            row.reel_no.clone(),
            row.frame_no.clone(),
            row.patent_count.to_string(),
            row.assignor_names.clone(),
            row.filing_attorney_name.clone().unwrap_or_default(),
            parcel_count,
            zeros,
            row.source_file.clone(),
            row.load_run_id.to_string(),
        ]));
        csv.push('\n');
    }
    fs::write(path, csv)
        .map_err(|err| StoreError::AssignmentLoad(format!("write csv {}: {err}", path.display())))?;
    let provenance_path = provenance_path(path);
    let exported_at: String = conn.query_row("SELECT datetime('now')", [], |row| row.get(0))?;
    let load_run_list = if load_runs.is_empty() {
        "none".to_string()
    } else {
        load_runs
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let source_file_list = if source_files.is_empty() {
        "none".to_string()
    } else {
        source_files.into_iter().collect::<Vec<_>>().join(", ")
    };
    let provenance = format!(
        "exported_at={exported_at}\nconfig_directory={}\nvertical_id={}\nconfig_version={}\nconfig_fingerprint={}\nload_run_ids={load_run_list}\nsource_files={source_file_list}\nactive_filters={}\nrow_count={}\n{CORRESPONDENT_ATTORNEY_SENTENCE}\n",
        meta.config_directory,
        config.manifest.vertical_id,
        config.manifest.config_version,
        meta.config_fingerprint,
        page.active_filters_text,
        page.rows.len(),
    );
    fs::write(&provenance_path, provenance).map_err(|err| {
        StoreError::AssignmentLoad(format!(
            "write provenance {}: {err}",
            provenance_path.display()
        ))
    })?;
    Ok(AssignmentExportResult {
        csv_path: path.display().to_string(),
        provenance_path: provenance_path.display().to_string(),
        rows_exported: page.rows.len() as i64,
    })
}

fn load_assignee_rows(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<Vec<AssignmentListRow>, StoreError> {
    let mut assignors: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut assignor_stmt =
        conn.prepare("SELECT reel_no, frame_no, sequence, name FROM assignment_assignors")?;
    let assignor_rows = assignor_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut assignor_raw: Vec<(String, String, i64, Option<String>)> = Vec::new();
    for row in assignor_rows {
        assignor_raw.push(row?);
    }
    assignor_raw.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (reel, frame, _, name) in assignor_raw {
        if let Some(name) = name {
            if !name.is_empty() {
                assignors.entry((reel, frame)).or_default().push(name);
            }
        }
    }

    let mut patent_counts: HashMap<(String, String), i64> = HashMap::new();
    let mut patent_stmt = conn.prepare(
        "SELECT reel_no, frame_no, COUNT(*) FROM assignment_properties GROUP BY reel_no, frame_no",
    )?;
    let patent_rows = patent_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in patent_rows {
        let (reel, frame, count) = row?;
        patent_counts.insert((reel, frame), count);
    }

    let parcels = parcel_index(conn)?;

    let mut stmt = conn.prepare(
        "SELECT a.reel_no, a.frame_no, a.sequence, a.name, a.city, a.address_1, a.address_2,
                a.postcode, r.conveyance_text, r.recorded_date, r.last_update_date,
                r.correspondent_name, r.load_run_id, r.source_file
         FROM assignment_assignees a
         JOIN assignment_records r ON r.reel_no = a.reel_no AND r.frame_no = a.frame_no",
    )?;
    let mapped = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, String>(13)?,
        ))
    })?;
    let mut rows = Vec::new();
    for row in mapped {
        let (
            reel_no,
            frame_no,
            sequence,
            company_name,
            city,
            address_1,
            address_2,
            postcode,
            conveyance_text,
            recorded_date,
            last_update_date,
            filing_attorney_name,
            load_run_id,
            source_file,
        ) = row?;
        let (county, county_unmatched) = derive_county_flags(&config.assignments, city.as_deref());
        let key = (reel_no.clone(), frame_no.clone());
        let assignor_names = assignors
            .get(&key)
            .map(|names| names.join("; "))
            .unwrap_or_default();
        let patent_count = patent_counts.get(&key).copied().unwrap_or(0);
        let normalised = normalise_owner_name(&company_name);
        let parcel = if normalised.is_empty() {
            None
        } else {
            parcels.get(&normalised).cloned()
        };
        rows.push(AssignmentListRow {
            reel_no,
            frame_no,
            sequence,
            company_name,
            city,
            county,
            county_unmatched,
            address_1,
            address_2,
            postcode,
            conveyance_text,
            recorded_date,
            last_update_date,
            patent_count,
            assignor_names,
            filing_attorney_name,
            parcel,
            load_run_id,
            source_file,
            latest_outcome: None,
        });
    }
    Ok(rows)
}

fn parcel_index(conn: &Connection) -> Result<HashMap<String, AssignmentParcelLink>, StoreError> {
    let mut parcels: HashMap<String, AssignmentParcelLink> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT owner_name, COUNT(*), SUM(CASE WHEN improvement_value = 0 THEN 1 ELSE 0 END)
         FROM current_parcels
         WHERE owner_name IS NOT NULL AND trim(owner_name) != ''
         GROUP BY owner_name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    for row in rows {
        let (owner, parcel_count, zeros) = row?;
        let key = normalise_owner_name(&owner);
        if key.is_empty() {
            continue;
        }
        let entry = parcels.entry(key.clone()).or_insert(AssignmentParcelLink {
            parcel_count: 0,
            improvement_value_zero_count: 0,
            group_key: key,
        });
        entry.parcel_count += parcel_count;
        entry.improvement_value_zero_count += zeros.unwrap_or(0);
    }
    Ok(parcels)
}

fn parcel_match_rate(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<AssignmentParcelMatchRate, StoreError> {
    let rows = load_assignee_rows(config, conn)?;
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut matched: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        if row.county.is_none() {
            continue;
        }
        let key = normalise_owner_name(&row.company_name);
        if key.is_empty() {
            continue;
        }
        names.insert(key.clone());
        if row.parcel.is_some() {
            matched.insert(key);
        }
    }
    let distinct_names = names.len() as i64;
    let matched_n = matched.len() as i64;
    let text = format!(
        "{matched_n} of {distinct_names} distinct names match an owner in current_parcels."
    );
    Ok(AssignmentParcelMatchRate {
        matched: matched_n,
        distinct_names,
        text,
    })
}

fn unmatched_wisconsin_cities(
    conn: &Connection,
    config: &radar_config::AssignmentsFile,
) -> Result<Vec<UnmatchedCity>, StoreError> {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT city, state FROM assignment_assignees")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    for row in rows {
        let (city, state) = row?;
        let city_val = city.as_deref().unwrap_or("").trim();
        let state_val = state.as_deref().unwrap_or("");
        if city_val.is_empty() {
            continue;
        }
        if config.is_wisconsin_state(state_val) && config.derive_county(city_val).is_none() {
            *counts.entry(city_val.to_string()).or_insert(0) += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|(city, count)| UnmatchedCity { city, count })
        .collect())
}

fn derive_county_flags(
    config: &radar_config::AssignmentsFile,
    city: Option<&str>,
) -> (Option<String>, bool) {
    let city = city.unwrap_or("").trim();
    if city.is_empty() {
        return (None, false);
    }
    match config.derive_county(city) {
        Some(county) => (Some(county.to_string()), false),
        None => (None, true),
    }
}

fn clean_query(query: &AssignmentListQuery) -> AssignmentListQuery {
    AssignmentListQuery {
        county: nonempty(query.county.clone()),
        city: nonempty(query.city.clone()),
        conveyance_text: nonempty(query.conveyance_text.clone()),
        recorded_date_from: nonempty(query.recorded_date_from.clone()),
        recorded_date_to: nonempty(query.recorded_date_to.clone()),
        last_update_date_from: nonempty(query.last_update_date_from.clone()),
        last_update_date_to: nonempty(query.last_update_date_to.clone()),
        name_contains: nonempty(query.name_contains.clone()),
        offset: query.offset,
        limit: query.limit,
    }
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn row_matches(row: &AssignmentListRow, query: &AssignmentListQuery) -> bool {
    if let Some(county) = query.county.as_deref() {
        if county.eq_ignore_ascii_case(COUNTY_UNMATCHED) {
            if !row.county_unmatched {
                return false;
            }
        } else if row.county.as_deref() != Some(county) {
            return false;
        }
    }
    if let Some(city) = query.city.as_deref() {
        if row.city.as_deref() != Some(city) {
            return false;
        }
    }
    if let Some(conveyance) = query.conveyance_text.as_deref() {
        if row.conveyance_text.as_deref() != Some(conveyance) {
            return false;
        }
    }
    if let Some(from) = query.recorded_date_from.as_deref() {
        match row.recorded_date.as_deref() {
            Some(date) if date >= from => {}
            _ => return false,
        }
    }
    if let Some(to) = query.recorded_date_to.as_deref() {
        match row.recorded_date.as_deref() {
            Some(date) if date <= to => {}
            _ => return false,
        }
    }
    if let Some(from) = query.last_update_date_from.as_deref() {
        match row.last_update_date.as_deref() {
            Some(date) if date >= from => {}
            _ => return false,
        }
    }
    if let Some(to) = query.last_update_date_to.as_deref() {
        match row.last_update_date.as_deref() {
            Some(date) if date <= to => {}
            _ => return false,
        }
    }
    if let Some(needle) = query.name_contains.as_deref() {
        let needle = normalise_owner_name(needle);
        if needle.is_empty() {
            return true;
        }
        if !normalise_owner_name(&row.company_name).contains(&needle) {
            return false;
        }
    }
    true
}

fn compare_rows(a: &AssignmentListRow, b: &AssignmentListRow) -> Ordering {
    b.recorded_date
        .cmp(&a.recorded_date)
        .then(a.reel_no.cmp(&b.reel_no))
        .then(a.frame_no.cmp(&b.frame_no))
        .then(a.sequence.cmp(&b.sequence))
}

fn active_filters_text(query: &AssignmentListQuery) -> String {
    let mut parts = Vec::new();
    if let Some(county) = query.county.as_deref() {
        parts.push(format!("county={county}"));
    }
    if let Some(city) = query.city.as_deref() {
        parts.push(format!("city={city}"));
    }
    if let Some(text) = query.conveyance_text.as_deref() {
        parts.push(format!("conveyance_text={text}"));
    }
    if let Some(from) = query.recorded_date_from.as_deref() {
        parts.push(format!("recorded_date_from={from}"));
    }
    if let Some(to) = query.recorded_date_to.as_deref() {
        parts.push(format!("recorded_date_to={to}"));
    }
    if let Some(from) = query.last_update_date_from.as_deref() {
        parts.push(format!("last_update_date_from={from}"));
    }
    if let Some(to) = query.last_update_date_to.as_deref() {
        parts.push(format!("last_update_date_to={to}"));
    }
    if let Some(name) = query.name_contains.as_deref() {
        parts.push(format!("name_contains={name}"));
    }
    if parts.is_empty() {
        "Active filters: none".to_string()
    } else {
        format!("Active filters: {}", parts.join("; "))
    }
}

fn provenance_path(csv_path: &Path) -> std::path::PathBuf {
    let stem = csv_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".to_string());
    match csv_path.parent() {
        Some(parent) => parent.join(format!("{stem}.provenance.txt")),
        None => Path::new(&format!("{stem}.provenance.txt")).to_path_buf(),
    }
}

fn csv_row(fields: &[String]) -> String {
    fields
        .iter()
        .map(|field| escape_csv(field))
        .collect::<Vec<_>>()
        .join(",")
}

fn escape_csv(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::migrate;
    use crate::rank::tests::{test_bands, test_config};
    use radar_config::sources::{SourceEntry, SourceFamily, SourceKind, SourceStatus};
    use rusqlite::params;
    use std::fs::File;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::FileOptions;
    use zip::ZipWriter;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (1, 'src-uspto-assignments', 'success', '2026-09-04')",
            [],
        )
        .expect("run");
        conn
    }

    fn config_with_cadence(days: Option<u32>) -> RadarConfig {
        let mut config = test_config(1, "caveat", test_bands());
        config.sources.sources = vec![SourceEntry {
            source_id: USPTO_ASSIGNMENTS_SOURCE_ID.to_string(),
            display_name: "USPTO patent assignment daily files".to_string(),
            family: SourceFamily::LegalPublicRecord,
            kind: SourceKind::ManualUpload,
            status: SourceStatus::Live,
            cadence_days: days,
            publication_lag_days: None,
            lag_measured: false,
            url: Some(USPTO_DOWNLOAD_PAGE.to_string()),
            url_verified: true,
            requires_key: false,
            attribution: None,
            notes: None,
        }];
        config
    }

    fn insert_assignment(
        conn: &Connection,
        reel: &str,
        frame: &str,
        recorded: &str,
        last_update: &str,
        conveyance: &str,
        source_file: &str,
        assignees: &[(&str, &str, &str, &str)],
        assignors: &[(&str, &str)],
        patents: &[(&str, &str, &str, &str)],
    ) {
        conn.execute(
            "INSERT INTO assignment_records (
                 reel_no, frame_no, recorded_date, last_update_date, conveyance_text,
                 correspondent_name, correspondent_address, source_file, load_run_id, fetched_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'LAW FIRM LLP', '1 MAIN', ?6, 1, '2026-09-04T00:00:00+00:00')",
            params![reel, frame, recorded, last_update, conveyance, source_file],
        )
        .expect("record");
        for (idx, (name, city, address, postcode)) in assignees.iter().enumerate() {
            conn.execute(
                "INSERT INTO assignment_assignees (
                     reel_no, frame_no, sequence, name, address_1, city, state, postcode
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'WI', ?7)",
                params![reel, frame, (idx as i64) + 1, name, address, city, postcode],
            )
            .expect("assignee");
        }
        for (idx, (name, executed)) in assignors.iter().enumerate() {
            conn.execute(
                "INSERT INTO assignment_assignors (
                     reel_no, frame_no, sequence, name, execution_date
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![reel, frame, (idx as i64) + 1, name, executed],
            )
            .expect("assignor");
        }
        for (idx, (country, doc, kind, title)) in patents.iter().enumerate() {
            conn.execute(
                "INSERT INTO assignment_properties (
                     reel_no, frame_no, sequence, country, doc_number, kind, invention_title
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![reel, frame, (idx as i64) + 1, country, doc, kind, title],
            )
            .expect("property");
        }
    }

    fn insert_parcel(conn: &Connection, taxkey: &str, owner: &str, improvement: f64) {
        conn.execute(
            "INSERT INTO current_parcels (
                 county_fips, muni_code, taxkey, id, load_run_id, owner_name,
                 improvement_value, materialised_at
             ) VALUES ('55133', 'WAKC', ?1, 1, 1, ?2, ?3, '2026-09-04')",
            params![taxkey, owner, improvement],
        )
        .expect("parcel");
    }

    fn xml_record(reel: &str, frame: &str, city: &str, name: &str) -> String {
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
      <state>WI</state>
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

    fn temp_folder() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "radar_assign_page_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("tmp");
        dir
    }

    #[test]
    fn list_returns_rows_and_reports_total() {
        let conn = open();
        insert_assignment(
            &conn,
            "1",
            "1",
            "2024-01-15",
            "2026-09-04",
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[("ACME LLC", "Waukesha", "1 ST", "53186")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "1", "A1", "Widget")],
        );
        insert_assignment(
            &conn,
            "1",
            "2",
            "2025-03-01",
            "2026-09-04",
            "MERGER",
            "ad20260904.zip",
            &[("ACME LLC", "Waukesha", "1 ST", "53186")],
            &[("INVENTOR TWO", "2025-01-01")],
            &[("US", "2", "B2", "Gadget")],
        );
        let page = list_assignments(
            &config_with_cadence(Some(7)),
            &conn,
            &AssignmentListQuery::default(),
        )
        .expect("page");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 2);
        assert_eq!(page.limit, DEFAULT_PAGE_LIMIT);
        assert_eq!(page.rows[0].reel_no, "1");
        assert_eq!(page.rows[0].frame_no, "2");
        assert_eq!(page.rows[0].company_name, "ACME LLC");
        assert_eq!(page.rows[0].filing_attorney_name.as_deref(), Some("LAW FIRM LLP"));
        assert_eq!(page.rows[0].patent_count, 1);
    }

    #[test]
    fn county_is_derived_and_unlisted_city_is_unmatched() {
        let conn = open();
        insert_assignment(
            &conn,
            "2",
            "1",
            "2024-01-15",
            "2026-09-04",
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[("DIRTY CITY LLC", "WAUKESH", "1 ST", "53186")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "1", "A1", "Widget")],
        );
        insert_assignment(
            &conn,
            "2",
            "2",
            "2024-02-01",
            "2026-09-04",
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[("CLEAN CITY LLC", "Waukesha", "2 ST", "53186")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "2", "A1", "Widget")],
        );
        let config = config_with_cadence(Some(7));
        let options = assignment_filter_options(&config, &conn).expect("options");
        assert!(options.counties.contains(&"waukesha".to_string()));
        assert!(options.counties.contains(&COUNTY_UNMATCHED.to_string()));
        let page = list_assignments(
            &config,
            &conn,
            &AssignmentListQuery {
                county: Some(COUNTY_UNMATCHED.to_string()),
                ..AssignmentListQuery::default()
            },
        )
        .expect("page");
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 1);
        assert!(page.rows[0].county_unmatched);
        assert_eq!(page.rows[0].company_name, "DIRTY CITY LLC");
        assert!(page.rows[0].county.is_none());
    }

    #[test]
    fn filter_narrows_count_and_reports_both() {
        let conn = open();
        insert_assignment(
            &conn,
            "3",
            "1",
            "2024-01-15",
            "2026-09-04",
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[("WAUKESHA CO LLC", "Waukesha", "1 ST", "53186")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "1", "A1", "Widget")],
        );
        insert_assignment(
            &conn,
            "3",
            "2",
            "2024-06-01",
            "2026-09-04",
            "MERGER",
            "ad20260904.zip",
            &[("MILWAUKEE CO LLC", "Milwaukee", "2 ST", "53202")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "2", "A1", "Widget")],
        );
        let page = list_assignments(
            &config_with_cadence(Some(7)),
            &conn,
            &AssignmentListQuery {
                city: Some("Milwaukee".to_string()),
                ..AssignmentListQuery::default()
            },
        )
        .expect("page");
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 1);
        assert_eq!(page.rows[0].company_name, "MILWAUKEE CO LLC");
        assert!(page.active_filters_text.contains("city=Milwaukee"));
    }

    #[test]
    fn detail_returns_every_assignee_on_the_assignment() {
        let conn = open();
        insert_assignment(
            &conn,
            "4",
            "9",
            "2024-01-15",
            "2026-09-04",
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[
                ("FIRST LLC", "Waukesha", "1 ST", "53186"),
                ("SECOND LLC", "Milwaukee", "2 ST", "53202"),
            ],
            &[("INVENTOR A", "2024-01-01"), ("INVENTOR B", "2024-01-02")],
            &[("US", "11", "A1", "Alpha"), ("US", "12", "B2", "Beta")],
        );
        let detail = assignment_detail(&config_with_cadence(Some(7)), &conn, "4", "9")
            .expect("detail");
        assert_eq!(detail.assignees.len(), 2);
        assert_eq!(detail.assignees[0].name, "FIRST LLC");
        assert_eq!(detail.assignees[1].name, "SECOND LLC");
        assert_eq!(detail.assignors.len(), 2);
        assert_eq!(detail.properties.len(), 2);
        assert_eq!(detail.filing_attorney_name.as_deref(), Some("LAW FIRM LLP"));
    }

    #[test]
    fn unmatched_parcel_name_shows_nothing_not_zero() {
        let conn = open();
        insert_assignment(
            &conn,
            "5",
            "1",
            "2024-01-15",
            "2026-09-04",
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[("NO PARCEL LLC", "Waukesha", "1 ST", "53186")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "1", "A1", "Widget")],
        );
        insert_parcel(&conn, "t1", "OTHER OWNER LLC", 0.0);
        let page = list_assignments(
            &config_with_cadence(Some(7)),
            &conn,
            &AssignmentListQuery::default(),
        )
        .expect("page");
        assert_eq!(page.rows.len(), 1);
        assert!(page.rows[0].parcel.is_none());
        assert_eq!(page.parcel_match.matched, 0);
        assert_eq!(page.parcel_match.distinct_names, 1);
    }

    #[test]
    fn refresh_on_empty_folder_names_the_folder_and_does_not_fail() {
        let mut conn = open();
        let dir = temp_folder();
        let result = refresh_assignments(&mut conn, &dir, &config_with_cadence(Some(7)).assignments)
            .expect("refresh");
        assert!(result.empty_folder);
        assert!(result.summary.contains(&dir.display().to_string()));
        assert!(result.summary.contains(USPTO_DOWNLOAD_PAGE));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_skips_already_loaded_file_and_counts_it() {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        let dir = temp_folder();
        let xml = wrap(&xml_record("8", "8", "Waukesha", "SKIP ME LLC"));
        write_zip(&dir, "ad20260904.zip", &xml);
        let first = refresh_assignments(&mut conn, &dir, &config_with_cadence(Some(7)).assignments)
            .expect("first");
        assert_eq!(first.files_skipped, 0);
        assert_eq!(first.wisconsin_stored, 1);
        let second = refresh_assignments(&mut conn, &dir, &config_with_cadence(Some(7)).assignments)
            .expect("second");
        assert_eq!(second.files_processed, 1);
        assert_eq!(second.files_skipped, 1);
        assert_eq!(second.wisconsin_stored, 0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM assignment_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn staleness_reads_cadence_from_the_registry() {
        let conn = open();
        let last_update: String = conn
            .query_row("SELECT date('now', '-10 days')", [], |row| row.get(0))
            .expect("date");
        insert_assignment(
            &conn,
            "6",
            "1",
            "2024-01-15",
            &last_update,
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[("STALE LLC", "Waukesha", "1 ST", "53186")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "1", "A1", "Widget")],
        );
        let tight = assignment_staleness(&conn, &config_with_cadence(Some(3)).sources).expect("3");
        assert_eq!(tight.cadence_days, Some(3));
        assert!(tight.stale, "10 days is more than a 3-day cadence");
        assert!(tight.text.contains(USPTO_DOWNLOAD_PAGE));
        let wide = assignment_staleness(&conn, &config_with_cadence(Some(30)).sources).expect("30");
        assert_eq!(wide.cadence_days, Some(30));
        assert!(
            !wide.stale,
            "10 days is not more than a 30-day cadence; 7 must not be hardcoded"
        );
    }

    #[test]
    fn export_writes_both_files_and_provenance_names_the_attorney() {
        let conn = open();
        insert_assignment(
            &conn,
            "7",
            "1",
            "2024-01-15",
            "2026-09-04",
            "ASSIGNMENT OF ASSIGNORS INTEREST",
            "ad20260904.zip",
            &[("EXPORT LLC", "Waukesha", "1 ST", "53186")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "1", "A1", "Widget")],
        );
        insert_assignment(
            &conn,
            "7",
            "2",
            "2024-06-01",
            "2026-09-04",
            "MERGER",
            "ad20260904.zip",
            &[("OTHER LLC", "Milwaukee", "2 ST", "53202")],
            &[("INVENTOR", "2024-01-01")],
            &[("US", "2", "A1", "Widget")],
        );
        let dir = temp_folder();
        let csv_path = dir.join("filtered.csv");
        let result = export_assignments(
            &config_with_cadence(Some(7)),
            &conn,
            &AssignmentListQuery {
                city: Some("Waukesha".to_string()),
                ..AssignmentListQuery::default()
            },
            &csv_path,
            &AssignmentExportMeta {
                config_directory: "config/dr_data_icp".to_string(),
                config_fingerprint: "test-fingerprint".to_string(),
            },
        )
        .expect("export");
        assert_eq!(result.rows_exported, 1);
        let csv = fs::read_to_string(&csv_path).expect("csv");
        assert!(csv.contains("EXPORT LLC"));
        assert!(!csv.contains("OTHER LLC"));
        let provenance = fs::read_to_string(&result.provenance_path).expect("prov");
        assert!(provenance.contains(CORRESPONDENT_ATTORNEY_SENTENCE));
        assert!(provenance.contains("config_directory=config/dr_data_icp"));
        assert!(provenance.contains("vertical_id=test"));
        assert!(provenance.contains("config_fingerprint=test-fingerprint"));
        assert!(provenance.contains("city=Waukesha"));
        assert!(provenance.contains("row_count=1"));
        assert!(provenance.contains("load_run_ids=1"));
        assert!(provenance.contains("source_files=ad20260904.zip"));
        let _ = fs::remove_dir_all(&dir);
    }
}
