// Milwaukee Accela occupancy. Two-stage pull per ADR-0011.
// List stage is a session POST. Rich fields come from CapDetail GET.
// Applicant street and Home Phone are out of schema.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::normalise::normalise_owner_name;
use crate::StoreError;

pub const ACCELA_OCCUPANCY_SOURCE_ID: &str = "src-milwaukee-accela-occupancy";
pub const RECORD_TYPE_OCCUPANCY: &str = "Occupancy";
pub const NEW_BUSINESS_REASON: &str = "1-Establish a new business or use";
pub const MILWAUKEE_COUNTY_FIPS: &str = "55079";
pub const VALUE_SOURCE_GRID: &str = "csv_grid";
pub const VALUE_SOURCE_DETAIL: &str = "detail_page";
pub const SEARCH_URL: &str =
    "https://aca-prod.accela.com/MILWAUKEE/Cap/CapHome.aspx?module=Building&TabName=Building";
pub const OCCUPANCY_TYPE_VALUE: &str = "Building/Occupancy/NA/NA";
pub const DETAIL_SLEEP_MS: u64 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupancyGridRow {
    pub record_number: String,
    pub record_type: String,
    pub record_date: Option<String>,
    pub status: Option<String>,
    pub project_name: Option<String>,
    pub site_address: Option<String>,
    pub short_notes: Option<String>,
    pub cap_id1: Option<String>,
    pub cap_id2: Option<String>,
    pub cap_id3: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OccupancyDetail {
    pub applicant_name: Option<String>,
    pub applicant_street: Option<String>,
    pub work_phone: Option<String>,
    pub home_phone: Option<String>,
    pub unlabeled_phone: Option<String>,
    pub unlabeled_phone_review: bool,
    pub owner_name: Option<String>,
    pub owner_mailing_address: Option<String>,
    pub legal_business_name: Option<String>,
    pub dba: Option<String>,
    pub reason_for_applying: Option<String>,
    pub business_type: Option<String>,
    pub hours: Option<String>,
    pub serving_food: Option<String>,
    pub serving_alcohol: Option<String>,
    pub parking: Option<String>,
    pub prior_use: Option<String>,
    pub vacancy_duration: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupancyRecord {
    pub grid: OccupancyGridRow,
    pub detail: Option<OccupancyDetail>,
    pub detail_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OccupancyLoadResult {
    pub load_run_id: Option<i64>,
    pub stored_rows: i64,
    pub skipped_existing: i64,
    pub excluded_non_occupancy: i64,
    pub detail_ok: i64,
    pub detail_failed: i64,
    pub parcel_matched: i64,
    pub parcel_unmatched: i64,
    pub match_text: String,
    pub snapshot_rows: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupancyFilterResult {
    pub occupancy: Vec<OccupancyGridRow>,
    pub excluded_non_occupancy: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupancyNewBusinessHit {
    pub record_number: String,
    pub legal_business_name: Option<String>,
    pub site_address: Option<String>,
    pub taxkey: Option<String>,
}

pub fn filter_occupancy_rows(rows: Vec<OccupancyGridRow>) -> OccupancyFilterResult {
    let mut occupancy = Vec::new();
    let mut excluded_non_occupancy = 0;
    for row in rows {
        if row.record_type.trim().eq_ignore_ascii_case(RECORD_TYPE_OCCUPANCY) {
            occupancy.push(row);
        } else {
            excluded_non_occupancy += 1;
        }
    }
    OccupancyFilterResult {
        occupancy,
        excluded_non_occupancy,
    }
}

pub fn apply_phone_privacy(mut detail: OccupancyDetail) -> OccupancyDetail {
    detail.home_phone = None;
    detail.applicant_street = None;
    detail
}

pub fn classify_phone_label(label: &str, number: &str) -> PhoneClass {
    let compact: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
    if compact.is_empty() {
        return PhoneClass::Skip;
    }
    let label = label.trim().trim_end_matches(':').trim();
    if label.eq_ignore_ascii_case("Work Phone") {
        PhoneClass::Work(compact)
    } else if label.eq_ignore_ascii_case("Home Phone") {
        PhoneClass::Home
    } else if label.eq_ignore_ascii_case("Mobile Phone") {
        PhoneClass::Skip
    } else {
        PhoneClass::Unlabeled(compact)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneClass {
    Work(String),
    Home,
    Unlabeled(String),
    Skip,
}

pub fn next_poll_start_date(newest_stored_iso: Option<&str>) -> Option<String> {
    let newest = newest_stored_iso?.trim();
    let (y, m, d) = parse_iso_date(newest)?;
    let (y, m, d) = add_days(y, m, d, 1)?;
    Some(format_accela_date(y, m, d))
}

pub fn record_date_to_iso(mmddyyyy: &str) -> Option<String> {
    let parts: Vec<&str> = mmddyyyy.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let m: u32 = parts[0].parse().ok()?;
    let d: u32 = parts[1].parse().ok()?;
    let y: i32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

pub fn normalise_occupancy_site_address(raw: &str) -> String {
    let primary = raw.split(',').next().unwrap_or(raw);
    normalise_owner_name(primary)
}

pub fn parse_search_result_rows(html: &str) -> Vec<OccupancyGridRow> {
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut rest = html;
    let marker = r#"class="ACA_TabRow"#;
    while let Some(idx) = rest.find(marker) {
        let row_start = rest[..idx].rfind("<tr").unwrap_or(idx);
        let slice = &rest[row_start..];
        let end = slice.find("</tr>").unwrap_or(slice.len().min(8000));
        let block = &slice[..end];
        rest = &rest[idx + marker.len()..];
        if let Some(row) = grid_row_from_block(block) {
            if seen.insert(row.record_number.clone()) {
                rows.push(row);
            }
        }
    }
    if rows.is_empty() {
        rows.extend(parse_search_result_rows_cap_fallback(html));
    }
    rows
}

fn parse_search_result_rows_cap_fallback(html: &str) -> Vec<OccupancyGridRow> {
    let mut rows = Vec::new();
    let marker = "CapDetail.aspx?";
    let mut rest = html;
    while let Some(idx) = rest.find(marker) {
        let row_start = rest[..idx]
            .rfind("<tr")
            .unwrap_or_else(|| rest[..idx].rfind("<a").unwrap_or(idx));
        let slice = &rest[row_start..];
        let end = slice.find("</tr>").unwrap_or(slice.len().min(4000));
        let block = &slice[..end];
        rest = &rest[idx + marker.len()..];
        if let Some(row) = grid_row_from_block(block) {
            rows.push(row);
        }
    }
    rows
}

fn grid_row_from_block(block: &str) -> Option<OccupancyGridRow> {
    let record_number = span_id_value(block, "_lblPermitNumber1")
        .or_else(|| span_id_exact(block, "_lblPermitNumber"))
        .or_else(|| {
            between(block, "hlPermitNumber", "</a>").map(|inner| {
                let start = inner
                    .find("\">")
                    .or_else(|| inner.find("'>"))
                    .map(|i| i + 2)
                    .unwrap_or(0);
                strip_tags(&inner[start..]).trim().to_string()
            })
        })
        .filter(|s| !s.is_empty())?;
    let (cap_id1, cap_id2, cap_id3) = cap_ids_from_block(block);
    let record_type = span_after(block, "_lblType").or_else(|| {
        if block.contains(">Occupancy<") {
            Some(RECORD_TYPE_OCCUPANCY.to_string())
        } else {
            None
        }
    });
    Some(OccupancyGridRow {
        record_number,
        record_type: record_type.unwrap_or_default(),
        record_date: span_id_value(block, "_lblUpdatedTime").or_else(|| first_date(block)),
        status: span_id_value(block, "_lblStatus"),
        project_name: span_id_value(block, "_lblProjectName"),
        site_address: span_id_value(block, "_lblAddress"),
        short_notes: span_id_value(block, "_lblShortNotes"),
        cap_id1,
        cap_id2,
        cap_id3,
    })
}

fn cap_ids_from_block(block: &str) -> (Option<String>, Option<String>, Option<String>) {
    let from_href = (
        query_param(block, "capID1"),
        query_param(block, "capID2"),
        query_param(block, "capID3"),
    );
    if from_href.0.is_some() && from_href.1.is_some() && from_href.2.is_some() {
        return from_href;
    }
    if let Some((a, b, c)) = hidden_record_id(block) {
        return (Some(a), Some(b), Some(c));
    }
    from_href
}

fn hidden_record_id(block: &str) -> Option<(String, String, String)> {
    let idx = block.find("RecordId")?;
    let rest = &block[idx..];
    let val_at = rest.find("value=")?;
    let after = &rest[val_at + 6..].trim_start_matches(['"', '\'']);
    let end = after
        .find('"')
        .or_else(|| after.find('\''))
        .unwrap_or(after.len().min(40));
    let raw = after[..end].trim();
    let parts: Vec<&str> = raw.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ))
}

fn span_id_exact(block: &str, suffix: &str) -> Option<String> {
    let needle = format!("{suffix}\">");
    let idx = block.find(&needle)?;
    if block[..idx].ends_with('1') {
        return None;
    }
    let rest = &block[idx + needle.len()..];
    let end = rest.find('<')?;
    let text = rest[..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(html_unescape(text))
    }
}

pub fn parse_detail_html(html: &str) -> OccupancyDetail {
    let mut detail = OccupancyDetail {
        applicant_name: {
            let first = class_value(html, "contactinfo_firstname");
            let last = class_value(html, "contactinfo_lastname");
            match (first, last) {
                (None, None) => None,
                (a, b) => Some(
                    format!(
                        "{} {}",
                        a.unwrap_or_default(),
                        b.unwrap_or_default()
                    )
                    .trim()
                    .to_string(),
                ),
            }
        },
        applicant_street: class_value(html, "contactinfo_addressline1"),
        ..OccupancyDetail::default()
    };
    let mut home_digits = HashSet::new();
    for (label, number) in phone_fields(html) {
        match classify_phone_label(&label, &number) {
            PhoneClass::Work(n) => detail.work_phone = Some(n),
            PhoneClass::Home => {
                let compact: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
                if !compact.is_empty() {
                    home_digits.insert(compact);
                }
            }
            PhoneClass::Unlabeled(n) => {
                detail.unlabeled_phone = Some(n);
                detail.unlabeled_phone_review = true;
            }
            PhoneClass::Skip => {}
        }
    }
    if let Some(work) = detail.work_phone.clone() {
        if home_digits.contains(&work) {
            detail.work_phone = None;
        }
    }
    if let Some(unlabeled) = detail.unlabeled_phone.clone() {
        if home_digits.contains(&unlabeled) || detail.work_phone.as_ref() == Some(&unlabeled) {
            detail.unlabeled_phone = None;
            detail.unlabeled_phone_review = false;
        }
    }
    if let Some(owner) = owner_block(html) {
        detail.owner_name = owner.0;
        detail.owner_mailing_address = owner.1;
    }
    let pairs = asi_pairs(html);
    detail.legal_business_name = pairs.get("Legal Business Name").cloned();
    detail.dba = pairs.get("Doing Business As").cloned();
    detail.reason_for_applying = pairs.get("Reason for applying").cloned();
    detail.business_type = pairs.get("Type of business or use").cloned();
    detail.hours = pairs
        .get("Proposed days and hours of operation")
        .cloned();
    detail.serving_food = pairs.get("Will you be serving or preparing food").cloned();
    detail.serving_alcohol = pairs
        .get("Will you be serving or selling alcoholic beverages")
        .cloned();
    detail.parking = pairs
        .get("Parking provided for customers, visitors and employees")
        .cloned();
    detail.prior_use = pairs
        .get("Prior use of this space by last tenant or owner")
        .cloned();
    detail.vacancy_duration = pairs
        .get("Is or was the space  recently vacant")
        .or_else(|| pairs.get("Is or was the space recently vacant"))
        .cloned();
    apply_phone_privacy(detail)
}

pub fn load_accela_occupancy(
    conn: &mut Connection,
    records: &[OccupancyRecord],
    fetched_at: &str,
) -> Result<OccupancyLoadResult, StoreError> {
    let filtered = filter_occupancy_rows(records.iter().map(|r| r.grid.clone()).collect());
    let excluded_non_occupancy = filtered.excluded_non_occupancy;
    let occupancy_keys: std::collections::BTreeSet<String> = filtered
        .occupancy
        .iter()
        .map(|r| r.record_number.clone())
        .collect();
    let kept: Vec<&OccupancyRecord> = records
        .iter()
        .filter(|r| occupancy_keys.contains(&r.grid.record_number))
        .collect();

    let mut stored_rows = 0i64;
    let mut skipped_existing = 0i64;
    let mut detail_ok = 0i64;
    let mut detail_failed = 0i64;
    let mut parcel_matched = 0i64;
    let mut parcel_unmatched = 0i64;

    if kept.is_empty() {
        return Ok(OccupancyLoadResult {
            load_run_id: None,
            stored_rows: 0,
            skipped_existing,
            excluded_non_occupancy,
            detail_ok: 0,
            detail_failed: 0,
            parcel_matched: 0,
            parcel_unmatched: 0,
            match_text: "0 of 0 occupancy site addresses match current_parcels.".to_string(),
            snapshot_rows: 0,
        });
    }

    conn.execute(
        "INSERT INTO load_runs (source_id, started_at, finished_at, status, records_requested, records_written, records_skipped)
         VALUES (?1, ?2, ?2, 'success', ?3, ?3, ?4)",
        params![
            ACCELA_OCCUPANCY_SOURCE_ID,
            fetched_at,
            kept.len() as i64,
            excluded_non_occupancy
        ],
    )?;
    let load_run_id = conn.last_insert_rowid();
    let parcel_index = occupancy_parcel_index(conn)?;
    let tx = conn.transaction()?;
    let mut snapshot_rows = 0i64;
    for rec in kept {
        let (value_source, detail_retry, detail) = if rec.detail_failed || rec.detail.is_none() {
            detail_failed += 1;
            (VALUE_SOURCE_GRID, 1i64, OccupancyDetail::default())
        } else {
            detail_ok += 1;
            (
                VALUE_SOURCE_DETAIL,
                0i64,
                rec.detail.clone().unwrap_or_default(),
            )
        };
        let site = rec.grid.site_address.clone();
        let (taxkey, matched) = match site.as_deref() {
            Some(addr) => {
                let key = normalise_occupancy_site_address(addr);
                match parcel_index.get(&key) {
                    Some(tax) => (Some(tax.clone()), 1i64),
                    None => (None, 0i64),
                }
            }
            None => (None, 0i64),
        };
        if matched == 1 {
            parcel_matched += 1;
        } else {
            parcel_unmatched += 1;
        }
        let iso = rec
            .grid
            .record_date
            .as_deref()
            .and_then(record_date_to_iso);
        insert_occupancy_snapshot(
            &tx,
            "accela_occupancy_loads",
            rec,
            load_run_id,
            fetched_at,
            value_source,
            detail_retry,
            &detail,
            iso.as_deref(),
            taxkey.as_deref(),
            matched,
        )?;
        snapshot_rows += 1;
        let exists: Option<String> = tx
            .query_row(
                "SELECT record_number FROM accela_occupancy WHERE record_number = ?1",
                [&rec.grid.record_number],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            skipped_existing += 1;
            continue;
        }
        insert_occupancy_snapshot(
            &tx,
            "accela_occupancy",
            rec,
            load_run_id,
            fetched_at,
            value_source,
            detail_retry,
            &detail,
            iso.as_deref(),
            taxkey.as_deref(),
            matched,
        )?;
        stored_rows += 1;
    }
    tx.commit()?;
    let considered = parcel_matched + parcel_unmatched;
    let match_text = format!(
        "{parcel_matched} of {considered} occupancy site addresses match current_parcels."
    );
    Ok(OccupancyLoadResult {
        load_run_id: Some(load_run_id),
        stored_rows,
        skipped_existing,
        excluded_non_occupancy,
        detail_ok,
        detail_failed,
        parcel_matched,
        parcel_unmatched,
        match_text,
        snapshot_rows,
    })
}

fn insert_occupancy_snapshot(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    rec: &OccupancyRecord,
    load_run_id: i64,
    fetched_at: &str,
    value_source: &str,
    detail_retry: i64,
    detail: &OccupancyDetail,
    iso: Option<&str>,
    taxkey: Option<&str>,
    matched: i64,
) -> Result<(), StoreError> {
    if table != "accela_occupancy" && table != "accela_occupancy_loads" {
        return Err(StoreError::Accela(format!("refusing insert into {table}")));
    }
    let sql = format!(
        "INSERT INTO {table} (
                 record_number, source_id, load_run_id, fetched_at, value_source, detail_retry,
                 record_date, record_type, status, project_name, site_address, short_notes,
                 cap_id1, cap_id2, cap_id3,
                 applicant_name, work_phone, unlabeled_phone, unlabeled_phone_review,
                 owner_name, owner_mailing_address, legal_business_name, dba, reason_for_applying,
                 business_type, hours, serving_food, serving_alcohol, parking, prior_use,
                 vacancy_duration, taxkey, parcel_matched
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6,
                 ?7, ?8, ?9, ?10, ?11, ?12,
                 ?13, ?14, ?15,
                 ?16, ?17, ?18, ?19,
                 ?20, ?21, ?22, ?23, ?24,
                 ?25, ?26, ?27, ?28, ?29, ?30,
                 ?31, ?32, ?33
             )"
    );
    tx.execute(
        &sql,
        params![
            rec.grid.record_number,
            ACCELA_OCCUPANCY_SOURCE_ID,
            load_run_id,
            fetched_at,
            value_source,
            detail_retry,
            iso,
            rec.grid.record_type,
            rec.grid.status,
            rec.grid.project_name,
            rec.grid.site_address,
            rec.grid.short_notes,
            rec.grid.cap_id1,
            rec.grid.cap_id2,
            rec.grid.cap_id3,
            detail.applicant_name,
            detail.work_phone,
            detail.unlabeled_phone,
            if detail.unlabeled_phone_review { 1i64 } else { 0 },
            detail.owner_name,
            detail.owner_mailing_address,
            detail.legal_business_name,
            detail.dba,
            detail.reason_for_applying,
            detail.business_type,
            detail.hours,
            detail.serving_food,
            detail.serving_alcohol,
            detail.parking,
            detail.prior_use,
            detail.vacancy_duration,
            taxkey,
            matched,
        ],
    )?;
    Ok(())
}

pub fn newest_stored_record_date(conn: &Connection) -> Result<Option<String>, StoreError> {
    let date: Option<String> = conn.query_row(
        "SELECT MAX(d) FROM (
             SELECT MAX(record_date) AS d FROM accela_occupancy
             UNION ALL
             SELECT MAX(record_date) AS d FROM accela_occupancy_loads
         )",
        [],
        |row| row.get(0),
    )?;
    Ok(date)
}

pub fn detect_occupancy_new_business(
    conn: &Connection,
) -> Result<Vec<OccupancyNewBusinessHit>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT record_number, legal_business_name, site_address, taxkey
         FROM accela_occupancy
         WHERE reason_for_applying = ?1
         ORDER BY record_date DESC, record_number",
    )?;
    let rows = stmt.query_map([NEW_BUSINESS_REASON], |row| {
        Ok(OccupancyNewBusinessHit {
            record_number: row.get(0)?,
            legal_business_name: row.get(1)?,
            site_address: row.get(2)?,
            taxkey: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn detect_occupancy_new_business_for_load(
    conn: &Connection,
    load_run_id: i64,
) -> Result<Vec<OccupancyNewBusinessHit>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT record_number, legal_business_name, site_address, taxkey
         FROM accela_occupancy_loads
         WHERE load_run_id = ?1 AND reason_for_applying = ?2
         ORDER BY record_date DESC, record_number",
    )?;
    let rows = stmt.query_map(params![load_run_id, NEW_BUSINESS_REASON], |row| {
        Ok(OccupancyNewBusinessHit {
            record_number: row.get(0)?,
            legal_business_name: row.get(1)?,
            site_address: row.get(2)?,
            taxkey: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn print_accela_load(result: &OccupancyLoadResult) {
    println!(
        "accela occupancy stored={} snapshot_rows={} skipped_existing={} excluded_non_occupancy={} detail_ok={} detail_failed={}",
        result.stored_rows,
        result.snapshot_rows,
        result.skipped_existing,
        result.excluded_non_occupancy,
        result.detail_ok,
        result.detail_failed
    );
    println!("{}", result.match_text);
}

fn occupancy_parcel_index(conn: &Connection) -> Result<BTreeMap<String, String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT taxkey, site_address FROM current_parcels WHERE county_fips = ?1",
    )?;
    let rows = stmt.query_map([MILWAUKEE_COUNTY_FIPS], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    let mut map = BTreeMap::new();
    for row in rows {
        let (taxkey, site) = row?;
        if let Some(site) = site {
            let key = normalise_occupancy_site_address(&site);
            if !key.is_empty() {
                map.entry(key).or_insert(taxkey);
            }
        }
    }
    Ok(map)
}

fn query_param(block: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let idx = block.find(&needle)?;
    let rest = &block[idx + needle.len()..];
    let end = rest
        .find('&')
        .or_else(|| rest.find('"'))
        .or_else(|| rest.find('\''))
        .unwrap_or(rest.len().min(32));
    let raw = rest[..end].replace("&amp;", "&");
    let raw = raw.trim_end_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

fn span_id_value(block: &str, suffix: &str) -> Option<String> {
    let idx = block.find(suffix)?;
    let rest = &block[idx..];
    let start = rest.find('>')?;
    let rest = &rest[start + 1..];
    let end = rest.find('<')?;
    let text = rest[..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(html_unescape(text))
    }
}

fn span_after(block: &str, suffix: &str) -> Option<String> {
    span_id_value(block, suffix)
}

fn between<'a>(hay: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = hay.find(start)?;
    let rest = &hay[i..];
    let j = rest.find(end)?;
    Some(&rest[..j])
}

fn first_date(block: &str) -> Option<String> {
    let bytes = block.as_bytes();
    let mut i = 0;
    while i + 10 <= bytes.len() {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2] == b'/'
            && bytes[i + 5] == b'/'
        {
            let s = std::str::from_utf8(&bytes[i..i + 10]).ok()?;
            if s.chars().filter(|c| *c == '/').count() == 2 {
                return Some(s.to_string());
            }
        }
        i += 1;
    }
    None
}

fn class_value(html: &str, class: &str) -> Option<String> {
    let needle = format!("{class}'>");
    let alt = format!("{class}\">");
    let rest = html
        .find(&needle)
        .map(|i| &html[i + needle.len()..])
        .or_else(|| html.find(&alt).map(|i| &html[i + alt.len()..]))?;
    let end = rest.find('<')?;
    let text = rest[..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(html_unescape(text))
    }
}

fn phone_fields(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    let marker = "ACA_PhoneNumberLTR";
    while let Some(idx) = rest.find(marker) {
        let prefix_start = idx.saturating_sub(200);
        let prefix = &rest[prefix_start..idx];
        let label = closest_phone_label(prefix);
        let after_p = &rest[idx..];
        if let Some(gt) = after_p.find('>') {
            let num = &after_p[gt + 1..];
            if let Some(lt) = num.find('<') {
                let number = num[..lt].trim();
                if !number.is_empty() {
                    out.push((label, number.to_string()));
                }
            }
        }
        rest = &rest[idx + marker.len()..];
    }
    out
}

fn closest_phone_label(prefix: &str) -> String {
    let Some(idx) = prefix.rfind("Phone:") else {
        return "Phone".to_string();
    };
    let through = &prefix[..idx + "Phone:".len()];
    let start = through.rfind('>').map(|i| i + 1).unwrap_or(0);
    let label = through[start..].trim().trim_end_matches(':').trim();
    if label.is_empty() {
        "Phone".to_string()
    } else {
        label.to_string()
    }
}

fn owner_block(html: &str) -> Option<(Option<String>, Option<String>)> {
    let idx = html.find("Owner:</span>")?;
    let rest = &html[idx..idx + 2500.min(html.len() - idx)];
    let mut lines = Vec::new();
    let mut search = rest;
    while let Some(p) = search.find("vertical-align:top'>") {
        let after = &search[p + "vertical-align:top'>".len()..];
        let end = after.find("</td>").unwrap_or(after.len().min(120));
        let raw = strip_tags(&after[..end]);
        let text = raw.trim();
        if !text.is_empty() && text != "*" {
            lines.push(text.to_string());
        }
        search = &after[end.min(after.len())..];
        if lines.len() >= 3 {
            break;
        }
    }
    if lines.is_empty() {
        return None;
    }
    let name = lines.first().cloned();
    let mailing = if lines.len() > 1 {
        Some(lines[1..].join(", "))
    } else {
        None
    };
    Some((name, mailing))
}

fn asi_pairs(html: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut rest = html;
    let marker = "ACA_SmLabelBolder font11px'>";
    while let Some(idx) = rest.find(marker) {
        let after = &rest[idx + marker.len()..];
        let end_label = match after.find('<') {
            Some(i) => i,
            None => break,
        };
        let label = after[..end_label].trim().trim_end_matches(':').trim();
        if let Some(val_mark) = after.find("ACA_SmLabel ACA_SmLabel_FontSize'>") {
            let val_src = &after[val_mark + "ACA_SmLabel ACA_SmLabel_FontSize'>".len()..];
            if let Some(end_val) = val_src.find('<') {
                let value = html_unescape(val_src[..end_val].trim());
                if !label.is_empty() {
                    map.insert(label.to_string(), value);
                }
            }
        }
        rest = &after[end_label..];
    }
    map
}

fn strip_tags(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            while chars.next().is_some_and(|c| c != '>') {}
        } else {
            out.push(ch);
        }
    }
    html_unescape(out.trim())
}

fn html_unescape(raw: &str) -> String {
    raw.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn parse_iso_date(iso: &str) -> Option<(i32, u32, u32)> {
    let parts: Vec<&str> = iso.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

fn format_accela_date(y: i32, m: u32, d: u32) -> String {
    format!("{m:02}/{d:02}/{y:04}")
}

fn add_days(y: i32, m: u32, d: u32, days: i32) -> Option<(i32, u32, u32)> {
    let mut y = y;
    let mut m = m as i32;
    let mut d = d as i32 + days;
    while d > days_in_month(y, m as u32)? as i32 {
        d -= days_in_month(y, m as u32)? as i32;
        m += 1;
        if m > 12 {
            m = 1;
            y += 1;
        }
    }
    while d < 1 {
        m -= 1;
        if m < 1 {
            m = 12;
            y -= 1;
        }
        d += days_in_month(y, m as u32)? as i32;
    }
    Some((y, m as u32, d as u32))
}

fn days_in_month(y: i32, m: u32) -> Option<u32> {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 => Some(if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            29
        } else {
            28
        }),
        _ => None,
    }
}

const ACCELA_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";
pub const DETAIL_URL: &str = "https://aca-prod.accela.com/MILWAUKEE/Cap/CapDetail.aspx";

pub fn fetch_and_load_accela_occupancy(
    conn: &mut Connection,
    start_mmddyyyy: &str,
    end_mmddyyyy: Option<&str>,
    fetched_at: &str,
) -> Result<OccupancyLoadResult, StoreError> {
    let records = fetch_occupancy_records(start_mmddyyyy, end_mmddyyyy)?;
    load_accela_occupancy(conn, &records, fetched_at)
}

pub fn fetch_occupancy_records(
    start_mmddyyyy: &str,
    end_mmddyyyy: Option<&str>,
) -> Result<Vec<OccupancyRecord>, StoreError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(90))
        .user_agent(ACCELA_UA)
        .build();
    let mut cookies = CookieJar::default();
    let search_html = search_occupancy_pages(&agent, &mut cookies, start_mmddyyyy, end_mmddyyyy)?;
    let grid_rows = parse_search_result_rows(&search_html);
    println!("search stage harvested={}", grid_rows.len());
    let mut occupancy = Vec::new();
    let mut excluded = Vec::new();
    for row in grid_rows {
        if row.record_type.trim().eq_ignore_ascii_case(RECORD_TYPE_OCCUPANCY) {
            occupancy.push(row);
        } else {
            excluded.push(row);
        }
    }
    println!(
        "search stage occupancy={} excluded_non_occupancy={}",
        occupancy.len(),
        excluded.len()
    );
    for (i, row) in occupancy.iter().take(3).enumerate() {
        println!(
            "harvest sample {} record={} date={} cap3={:?}",
            i + 1,
            row.record_number,
            row.record_date.as_deref().unwrap_or("(null)"),
            row.cap_id3
        );
    }
    let mut records = Vec::new();
    let n = occupancy.len();
    for (i, row) in occupancy.into_iter().enumerate() {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(DETAIL_SLEEP_MS));
        }
        let (detail, failed) = match fetch_detail_page(&agent, &cookies, &row) {
            Ok(html) => (Some(parse_detail_html(&html)), false),
            Err(_) => (None, true),
        };
        if failed {
            println!(
                "detail GET failed {}/{} {}",
                i + 1,
                n,
                row.record_number
            );
        } else if (i + 1) % 10 == 0 || i + 1 == n {
            println!("detail GET ok {}/{}", i + 1, n);
        }
        records.push(OccupancyRecord {
            grid: row,
            detail,
            detail_failed: failed,
        });
    }
    for row in excluded {
        records.push(OccupancyRecord {
            grid: row,
            detail: None,
            detail_failed: true,
        });
    }
    Ok(records)
}

fn search_occupancy_pages(
    agent: &ureq::Agent,
    cookies: &mut CookieJar,
    start: &str,
    end: Option<&str>,
) -> Result<String, StoreError> {
    let get = request_get(agent, cookies, SEARCH_URL)?;
    cookies.absorb(&get);
    println!("search cookies={}", cookies.map.len());
    let mut html = get
        .into_string()
        .map_err(|err| StoreError::Accela(format!("search GET body: {err}")))?;
    let mut form = parse_form_fields(&html);
    println!(
        "search GET bytes={} form_fields={} has_viewstate={}",
        html.len(),
        form.len(),
        form.contains_key("__VIEWSTATE")
    );
    form.insert(
        "__EVENTTARGET".to_string(),
        "ctl00$PlaceHolderMain$btnNewSearch".to_string(),
    );
    form.insert("__EVENTARGUMENT".to_string(), String::new());
    form.insert(
        "ctl00$PlaceHolderMain$ddlSearchType".to_string(),
        "0".to_string(),
    );
    form.insert(
        "ctl00$PlaceHolderMain$generalSearchForm$ddlGSPermitType".to_string(),
        OCCUPANCY_TYPE_VALUE.to_string(),
    );
    form.insert(
        "ctl00$PlaceHolderMain$generalSearchForm$txtGSStartDate".to_string(),
        start.to_string(),
    );
    form.insert(
        "ctl00$PlaceHolderMain$generalSearchForm$txtGSEndDate".to_string(),
        end.unwrap_or("").to_string(),
    );
    html = post_form(agent, cookies, &form)?;
    println!(
        "search POST bytes={} showing={} no_record={} cap_detail_links={} cap_detail_q={}",
        html.len(),
        html.contains("Showing"),
        html.to_ascii_lowercase().contains("no record"),
        html.matches("CapDetail.aspx").count(),
        html.matches("CapDetail.aspx?").count()
    );
    if let Some(idx) = html.find("CapDetail.aspx") {
        let start = idx.saturating_sub(80);
        let end = html.len().min(idx + 220);
        println!("search POST cap snippet={}", html[start..end].replace('\n', " "));
    }
    let mut combined = html.clone();
    let mut pages = 0;
    while pages < 40 {
        pages += 1;
        let Some(target) = next_page_event_target(&html) else {
            break;
        };
        std::thread::sleep(std::time::Duration::from_millis(DETAIL_SLEEP_MS));
        let mut next = parse_form_fields(&html);
        next.insert("__EVENTTARGET".to_string(), target);
        next.insert("__EVENTARGUMENT".to_string(), String::new());
        html = post_form(agent, cookies, &next)?;
        combined.push_str(&html);
    }
    Ok(combined)
}

fn request_get(
    agent: &ureq::Agent,
    cookies: &CookieJar,
    url: &str,
) -> Result<ureq::Response, StoreError> {
    let mut req = agent.get(url).set("Accept", "text/html");
    if let Some(header) = cookies.header() {
        req = req.set("Cookie", &header);
    }
    req.call()
        .map_err(|err| StoreError::Accela(format!("GET {url}: {err}")))
}

fn post_form(
    agent: &ureq::Agent,
    cookies: &mut CookieJar,
    form: &BTreeMap<String, String>,
) -> Result<String, StoreError> {
    let pairs: Vec<(&str, &str)> = form
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut req = agent
        .post(SEARCH_URL)
        .set("Accept", "text/html")
        .set("Referer", SEARCH_URL);
    if let Some(header) = cookies.header() {
        req = req.set("Cookie", &header);
    }
    let resp = req
        .send_form(&pairs)
        .map_err(|err| StoreError::Accela(format!("search POST: {err}")))?;
    cookies.absorb(&resp);
    resp.into_string()
        .map_err(|err| StoreError::Accela(format!("search POST body: {err}")))
}

fn fetch_detail_page(
    agent: &ureq::Agent,
    cookies: &CookieJar,
    row: &OccupancyGridRow,
) -> Result<String, StoreError> {
    let (Some(a), Some(b), Some(c)) = (&row.cap_id1, &row.cap_id2, &row.cap_id3) else {
        return Err(StoreError::Accela(format!(
            "missing cap IDs for {}",
            row.record_number
        )));
    };
    let url = format!(
        "{DETAIL_URL}?Module=Building&TabName=Building&capID1={a}&capID2={b}&capID3={c}&agencyCode=MILWAUKEE"
    );
    let resp = request_get(agent, cookies, &url)?;
    resp.into_string()
        .map_err(|err| StoreError::Accela(format!("detail body {}: {err}", row.record_number)))
}

#[derive(Default)]
struct CookieJar {
    map: BTreeMap<String, String>,
}

impl CookieJar {
    fn absorb(&mut self, resp: &ureq::Response) {
        for raw in resp.all("set-cookie") {
            if let Some((name, value)) = parse_set_cookie(raw) {
                self.map.insert(name, value);
            }
        }
    }

    fn header(&self) -> Option<String> {
        if self.map.is_empty() {
            return None;
        }
        Some(
            self.map
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

fn parse_set_cookie(raw: &str) -> Option<(String, String)> {
    let pair = raw.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    if name.is_empty() {
        None
    } else {
        Some((name.to_string(), value.to_string()))
    }
}

fn parse_form_fields(html: &str) -> BTreeMap<String, String> {
    let mut data = BTreeMap::new();
    let mut rest = html;
    while let Some(idx) = find_tag_open(rest, "input") {
        let tag_end = rest[idx..].find('>').map(|i| idx + i + 1).unwrap_or(rest.len());
        let tag = &rest[idx..tag_end];
        rest = &rest[tag_end..];
        let typ = attr(tag, "type").unwrap_or_else(|| "text".to_string());
        if typ.eq_ignore_ascii_case("submit")
            || typ.eq_ignore_ascii_case("button")
            || typ.eq_ignore_ascii_case("image")
        {
            continue;
        }
        let Some(name) = attr(tag, "name") else {
            continue;
        };
        let value = attr(tag, "value").unwrap_or_default();
        data.entry(name).or_insert(value);
    }
    rest = html;
    while let Some(idx) = find_tag_open(rest, "select") {
        let tag_end = rest[idx..].find('>').map(|i| idx + i + 1).unwrap_or(rest.len());
        let tag = &rest[idx..tag_end];
        let Some(close) = find_ci(&rest[tag_end..], "</select>") else {
            break;
        };
        let inner = &rest[tag_end..tag_end + close];
        rest = &rest[tag_end + close + 9..];
        let Some(name) = attr(tag, "name") else {
            continue;
        };
        let value = selected_option(inner);
        data.entry(name).or_insert(value);
    }
    rest = html;
    while let Some(idx) = find_tag_open(rest, "textarea") {
        let tag_end = rest[idx..].find('>').map(|i| idx + i + 1).unwrap_or(rest.len());
        let tag = &rest[idx..tag_end];
        let Some(name) = attr(tag, "name") else {
            rest = &rest[tag_end..];
            continue;
        };
        let close = find_ci(&rest[tag_end..], "</textarea>").unwrap_or(0);
        let value = html_unescape(rest[tag_end..tag_end + close].trim());
        rest = &rest[tag_end + close.saturating_add(11)..];
        data.entry(name).or_insert(value);
    }
    data
}

fn find_tag_open(html: &str, name: &str) -> Option<usize> {
    let lower = format!("<{name}");
    let upper = format!("<{}", name.to_ascii_uppercase());
    match (html.find(&lower), html.find(&upper)) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn attr(tag: &str, key: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{key}={quote}");
        if let Some(idx) = tag.find(&needle) {
            let rest = &tag[idx + needle.len()..];
            if let Some(end) = rest.find(quote) {
                return Some(html_unescape(&rest[..end]));
            }
        }
        let needle = format!("{}={quote}", key.to_ascii_uppercase());
        if let Some(idx) = tag.find(&needle) {
            let rest = &tag[idx + needle.len()..];
            if let Some(end) = rest.find(quote) {
                return Some(html_unescape(&rest[..end]));
            }
        }
    }
    None
}

fn selected_option(inner: &str) -> String {
    let mut search = inner;
    while let Some(idx) = search.find("<option") {
        let end = search[idx..].find('>').map(|i| idx + i + 1).unwrap_or(search.len());
        let tag = &search[idx..end];
        if tag.to_ascii_lowercase().contains("selected") {
            return attr(tag, "value").unwrap_or_default();
        }
        search = &search[end..];
    }
    if let Some(idx) = inner.find("<option") {
        let end = inner[idx..].find('>').map(|i| idx + i + 1).unwrap_or(inner.len());
        return attr(&inner[idx..end], "value").unwrap_or_default();
    }
    String::new()
}

pub fn next_page_event_target(html: &str) -> Option<String> {
    let idx = html.find("SelectedPageButton")?;
    let window = &html[idx..html.len().min(idx + 4000)];
    let marker = "javascript:__doPostBack(&#39;";
    let alt = "javascript:__doPostBack('";
    let (hit, skip) = if let Some(i) = window.find(marker) {
        (i, marker.len())
    } else if let Some(i) = window.find(alt) {
        (i, alt.len())
    } else {
        return None;
    };
    let after = &window[hit + skip..];
    let end = after.find("&#39;").or_else(|| after.find('\''))?;
    Some(html_unescape(&after[..end]))
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::migrate::migrate;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    fn grid(number: &str, type_name: &str, date: &str, address: &str) -> OccupancyGridRow {
        OccupancyGridRow {
            record_number: number.to_string(),
            record_type: type_name.to_string(),
            record_date: Some(date.to_string()),
            status: Some("Open".to_string()),
            project_name: None,
            site_address: Some(address.to_string()),
            short_notes: None,
            cap_id1: Some("26CAP".to_string()),
            cap_id2: Some("00000".to_string()),
            cap_id3: Some("08GP4".to_string()),
        }
    }

    #[test]
    fn work_phone_is_kept_and_home_phone_is_dropped() {
        let html = r#"
            <span class='contactinfo_firstname'>Jimmy </span><span class='contactinfo_lastname'>Simms </span>
            <span class='contactinfo_addressline1'>2221 N. Humboldt Blvd.</span>
            <td>Work Phone:</td><td><div class="ACA_PhoneNumberLTR">4145541013</div></td>
            <span class='ACA_SmLabelBolder font11px'>Legal Business Name: </span>
            </div><div class='MoreDetail_ItemColASI MoreDetail_ItemCol2'>
            <span class='ACA_SmLabel ACA_SmLabel_FontSize'>I got tea cafe</span>
        "#;
        let detail = parse_detail_html(html);
        assert_eq!(detail.work_phone.as_deref(), Some("4145541013"));
        assert_eq!(detail.home_phone, None);
        assert_eq!(detail.applicant_street, None);
        assert_eq!(detail.applicant_name.as_deref(), Some("Jimmy Simms"));
        assert_eq!(detail.legal_business_name.as_deref(), Some("I got tea cafe"));
        assert!(!html.is_empty());
        let home_html = html.replace("Work Phone:", "Home Phone:");
        let home = parse_detail_html(&home_html);
        assert_eq!(home.work_phone, None);
        assert_eq!(home.home_phone, None);
        assert_eq!(home.unlabeled_phone, None);
    }

    #[test]
    fn unlabeled_phone_is_flagged_not_guessed() {
        let html = r#"
            <td>Fax Phone:</td><td><div class="ACA_PhoneNumberLTR">4140001111</div></td>
        "#;
        let detail = parse_detail_html(html);
        assert_eq!(detail.work_phone, None);
        assert_eq!(detail.home_phone, None);
        assert_eq!(detail.unlabeled_phone.as_deref(), Some("4140001111"));
        assert!(detail.unlabeled_phone_review);
    }

    #[test]
    fn home_phone_label_is_not_stored_on_load() {
        let mut conn = open();
        let mut detail = OccupancyDetail::default();
        detail.applicant_name = Some("Michelle Rodgers".to_string());
        detail.work_phone = None;
        let rec = OccupancyRecord {
            grid: grid(
                "NOCC-26-01549",
                RECORD_TYPE_OCCUPANCY,
                "09/17/2026",
                "5919 W VLIET ST, MILWAUKEE WI 53208",
            ),
            detail: Some(detail),
            detail_failed: false,
        };
        load_accela_occupancy(&mut conn, &[rec], "2026-09-17T12:00:00Z").expect("load");
        let phone: Option<String> = conn
            .query_row(
                "SELECT work_phone FROM accela_occupancy WHERE record_number = 'NOCC-26-01549'",
                [],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(phone, None);
        let cols: String = conn
            .prepare("PRAGMA table_info(accela_occupancy)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|v| v.unwrap())
            .collect::<Vec<_>>()
            .join(",");
        assert!(!cols.contains("home_phone"));
        assert!(!cols.contains("applicant_street"));
        assert!(!cols.contains("applicant_address"));
    }

    #[test]
    fn since_poll_uses_day_after_newest_stored() {
        assert_eq!(
            next_poll_start_date(Some("2026-09-15")),
            Some("09/16/2026".to_string())
        );
        assert_eq!(next_poll_start_date(None), None);
        let mut conn = open();
        let rec = OccupancyRecord {
            grid: grid(
                "NOCC-26-01500",
                RECORD_TYPE_OCCUPANCY,
                "09/15/2026",
                "100 E WISCONSIN AV, MILWAUKEE WI",
            ),
            detail: Some(OccupancyDetail::default()),
            detail_failed: false,
        };
        load_accela_occupancy(&mut conn, &[rec.clone()], "2026-09-17T12:00:00Z").expect("first");
        let newest = newest_stored_record_date(&conn).expect("newest");
        assert_eq!(newest.as_deref(), Some("2026-09-15"));
        let start = next_poll_start_date(newest.as_deref());
        assert_eq!(start.as_deref(), Some("09/16/2026"));
        let again = load_accela_occupancy(&mut conn, &[rec], "2026-09-17T13:00:00Z").expect("rerun");
        assert_eq!(again.stored_rows, 0);
        assert_eq!(again.skipped_existing, 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM accela_occupancy", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn non_occupancy_is_excluded_and_counted() {
        let mut conn = open();
        let occ = OccupancyRecord {
            grid: grid(
                "NOCC-26-01000",
                RECORD_TYPE_OCCUPANCY,
                "09/10/2026",
                "1 N WATER ST, MILWAUKEE WI",
            ),
            detail: Some(OccupancyDetail::default()),
            detail_failed: false,
        };
        let other = OccupancyRecord {
            grid: grid(
                "COM-ALT-26-00001",
                "Commercial Alteration Permit",
                "09/10/2026",
                "1 N WATER ST, MILWAUKEE WI",
            ),
            detail: None,
            detail_failed: true,
        };
        let result =
            load_accela_occupancy(&mut conn, &[occ, other], "2026-09-17T12:00:00Z").expect("load");
        assert_eq!(result.excluded_non_occupancy, 1);
        assert_eq!(result.stored_rows, 1);
        let types: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accela_occupancy WHERE record_type != 'Occupancy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(types, 0);
    }

    #[test]
    fn failed_detail_is_stored_for_retry_not_dropped() {
        let mut conn = open();
        let rec = OccupancyRecord {
            grid: grid(
                "NOCC-26-01001",
                RECORD_TYPE_OCCUPANCY,
                "09/11/2026",
                "2 N WATER ST, MILWAUKEE WI",
            ),
            detail: None,
            detail_failed: true,
        };
        let result = load_accela_occupancy(&mut conn, &[rec], "2026-09-17T12:00:00Z").expect("load");
        assert_eq!(result.stored_rows, 1);
        assert_eq!(result.detail_failed, 1);
        let (source, retry, legal): (String, i64, Option<String>) = conn
            .query_row(
                "SELECT value_source, detail_retry, legal_business_name
                 FROM accela_occupancy WHERE record_number = 'NOCC-26-01001'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(source, VALUE_SOURCE_GRID);
        assert_eq!(retry, 1);
        assert_eq!(legal, None);
    }

    #[test]
    fn parcel_join_is_honest_and_new_business_reason_detects() {
        let mut conn = open();
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (1, 'src-milwaukee-parcels', 'success', '2026-09-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO current_parcels (
                 county_fips, muni_code, taxkey, id, load_run_id, site_address, materialised_at
             ) VALUES ('55079', 'MKE', '123', 1, 1, '5919 W VLIET ST', '2026-09-01')",
            [],
        )
        .unwrap();
        let mut detail = OccupancyDetail::default();
        detail.reason_for_applying = Some(NEW_BUSINESS_REASON.to_string());
        detail.legal_business_name = Some("Lavish Waists".to_string());
        let matched = OccupancyRecord {
            grid: grid(
                "NOCC-26-01549",
                RECORD_TYPE_OCCUPANCY,
                "09/17/2026",
                "5919 W VLIET ST, MILWAUKEE WI 532082166",
            ),
            detail: Some(detail.clone()),
            detail_failed: false,
        };
        let unmatched = OccupancyRecord {
            grid: grid(
                "NOCC-26-01548",
                RECORD_TYPE_OCCUPANCY,
                "09/17/2026",
                "9999 NOWHERE RD, MILWAUKEE WI",
            ),
            detail: Some(detail),
            detail_failed: false,
        };
        let result =
            load_accela_occupancy(&mut conn, &[matched, unmatched], "2026-09-17T12:00:00Z")
                .expect("load");
        assert_eq!(result.parcel_matched, 1);
        assert_eq!(result.parcel_unmatched, 1);
        assert_eq!(
            result.match_text,
            "1 of 2 occupancy site addresses match current_parcels."
        );
        let hits = detect_occupancy_new_business(&conn).expect("detect");
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.taxkey.as_deref() == Some("123")));
        assert!(hits.iter().any(|h| h.taxkey.is_none()));
    }

    #[test]
    fn search_html_harvests_cap_ids_and_skips_nothing_on_occupancy() {
        let html = r#"
            <a id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_hlPermitNumber"
               href="/MILWAUKEE/Cap/CapDetail.aspx?Module=Building&amp;TabName=Building&amp;capID1=26CAP&amp;capID2=00000&amp;capID3=08GP4&amp;agencyCode=MILWAUKEE">
               <strong>NOCC-26-01548</strong>
            </a>
            <span>Occupancy</span>
            <span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_lblStatus">Open</span>
            <span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_lblAddress">5919 W VLIET ST, MILWAUKEE WI 532082166</span>
            09/17/2026
            </tr>
        "#;
        let rows = parse_search_result_rows(html);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].record_number, "NOCC-26-01548");
        assert_eq!(rows[0].cap_id1.as_deref(), Some("26CAP"));
        assert_eq!(rows[0].cap_id3.as_deref(), Some("08GP4"));
        assert_eq!(rows[0].record_date.as_deref(), Some("09/17/2026"));
    }

    #[test]
    fn next_page_target_is_the_link_after_the_selected_page() {
        let html = r#"
            <span class="SelectedPageButton font11px">1</span></td>
            <td class="aca_pagination_td"><a href="javascript:__doPostBack(&#39;ctl00$PlaceHolderMain$dgvPermitList$gdvPermitList$ctl23$ctl03&#39;,&#39;&#39;)">2</a></td>
        "#;
        assert_eq!(
            next_page_event_target(html).as_deref(),
            Some("ctl00$PlaceHolderMain$dgvPermitList$gdvPermitList$ctl23$ctl03")
        );
        assert_eq!(next_page_event_target("no pager"), None);
        assert_eq!(
            classify_phone_label("Work Phone", "(414) 555-0100"),
            PhoneClass::Work("4145550100".to_string())
        );
        assert_eq!(
            classify_phone_label("Home Phone", "4145550199"),
            PhoneClass::Home
        );
        assert_eq!(
            classify_phone_label("Fax", "4145550000"),
            PhoneClass::Unlabeled("4145550000".to_string())
        );
        assert_eq!(
            classify_phone_label("Mobile Phone", "4145550000"),
            PhoneClass::Skip
        );
    }

    #[test]
    fn home_phone_labeled_field_is_never_stored_under_any_column() {
        let html = r#"
            <td style='vertical-align:top;'>Home Phone:</td><td style='vertical-align:top;'><div class="ACA_PhoneNumberLTR">4145349700</div></td>
            <td style='vertical-align:top;'>Work Phone:</td><td style='vertical-align:top;'><div class="ACA_PhoneNumberLTR">4145349700</div></td>
            <td style='vertical-align:top;'>Mobile Phone:</td><td style='vertical-align:top;'><div class="ACA_PhoneNumberLTR">4145349700</div></td>
        "#;
        let detail = parse_detail_html(html);
        assert_eq!(detail.home_phone, None);
        assert_eq!(detail.work_phone, None);
        assert_eq!(detail.unlabeled_phone, None);
        assert!(!detail.unlabeled_phone_review);
        let mut conn = open();
        let rec = OccupancyRecord {
            grid: grid(
                "NOCC-26-01549",
                RECORD_TYPE_OCCUPANCY,
                "09/17/2026",
                "5919 W VLIET ST, MILWAUKEE WI 53208",
            ),
            detail: Some(detail),
            detail_failed: false,
        };
        load_accela_occupancy(&mut conn, &[rec], "2026-09-17T12:00:00Z").expect("load");
        let (work, unlabeled, review): (Option<String>, Option<String>, i64) = conn
            .query_row(
                "SELECT work_phone, unlabeled_phone, unlabeled_phone_review
                 FROM accela_occupancy WHERE record_number = 'NOCC-26-01549'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("row");
        assert_eq!(work, None);
        assert_eq!(unlabeled, None);
        assert_eq!(review, 0);
        let snapshot: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT work_phone, unlabeled_phone FROM accela_occupancy_loads
                 WHERE record_number = 'NOCC-26-01549'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("snapshot");
        assert_eq!(snapshot, (None, None));
        let cols: String = conn
            .prepare("PRAGMA table_info(accela_occupancy)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|v| v.unwrap())
            .collect::<Vec<_>>()
            .join(",");
        assert!(!cols.contains("home_phone"));
        assert!(!cols.to_ascii_lowercase().contains("4145349700"));
    }

    #[test]
    fn repeated_phone_label_same_as_work_is_not_unlabeled() {
        let html = r#"
            <td>Work Phone:</td><td><div class="ACA_PhoneNumberLTR">4145541013</div></td>
            <td>Phone:</td><td><div class="ACA_PhoneNumberLTR">4145541013</div></td>
        "#;
        let detail = parse_detail_html(html);
        assert_eq!(detail.work_phone.as_deref(), Some("4145541013"));
        assert_eq!(detail.unlabeled_phone, None);
        assert!(!detail.unlabeled_phone_review);
    }

    #[test]
    fn date_parser_matches_live_accela_lblupdatedtime_markup() {
        let html = r#"
<tr class="ACA_TabRow_Odd ACA_TabRow_Odd_FontSize">
<td><span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_lblUpdatedTime" CSS="ACA_SmLabel ACA_SmLabel_FontSize">09/17/2026</span></td>
<td>
<a id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_hlPermitNumber" href="/MILWAUKEE/Cap/CapDetail.aspx?Module=Building&amp;TabName=Building&amp;capID1=26CAP&amp;capID2=00000&amp;capID3=08GPO&amp;agencyCode=MILWAUKEE">
<span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_lblPermitNumber1">NOCC-26-01549</span>
</a>
<input type="hidden" ID="RecordId" value="26CAP-00000-08GPO" />
</td>
<td><span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_lblType">Occupancy</span></td>
<td><span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl02_lblAddress">5919 W VLIET ST, MILWAUKEE WI 532082166</span></td>
</tr>
<tr class="ACA_TabRow_Even ACA_TabRow_Even_FontSize">
<td><span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl09_lblUpdatedTime" CSS="ACA_SmLabel ACA_SmLabel_FontSize">09/16/2026</span></td>
<td><strong><span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl09_lblPermitNumber">26TMP-043818</span></strong>
<input type="hidden" ID="RecordId" value="26EST-00000-43860" /></td>
<td><span id="ctl00_PlaceHolderMain_dgvPermitList_gdvPermitList_ctl09_lblType">Occupancy</span></td>
</tr>
"#;
        let rows = parse_search_result_rows(html);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].record_number, "NOCC-26-01549");
        assert_eq!(rows[0].record_date.as_deref(), Some("09/17/2026"));
        assert_eq!(rows[0].cap_id3.as_deref(), Some("08GPO"));
        assert_eq!(rows[1].record_number, "26TMP-043818");
        assert_eq!(rows[1].record_date.as_deref(), Some("09/16/2026"));
        assert_eq!(rows[1].cap_id1.as_deref(), Some("26EST"));
        assert_eq!(rows[1].cap_id3.as_deref(), Some("43860"));
    }
}
