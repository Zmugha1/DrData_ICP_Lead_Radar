// The file lists currently active licences and is overwritten
// nightly. It carries no licence number and no new-versus-renewed
// flag. A first-time licence and an annual renewal both appear as a
// row with a fresh effective date.
// A row appearing today and absent yesterday is new to the file. It
// is usually a new licence and may be a record correction.
// The first snapshot establishes a baseline and produces no signal.
// Diff keys are (taxkey, lic_type). That pair is not unique in the
// published file. Without a licence number the detector treats the
// set of licensee strings on that pair as the identity of the row.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::StoreError;

pub const LIQUOR_SOURCE_ID: &str = "src-milwaukee-liquor-licences";
pub const LIQUOR_CSV_URL: &str = "https://data.milwaukee.gov/dataset/1aba8821-f5f6-4031-b469-5c39c90e99c7/resource/45c027b5-fa66-4de2-aa7e-d9314292093d/download/liquorlicenses.csv";
pub const PLACEHOLDER_TAXKEY: &str = "0000000000";
pub const MILWAUKEE_COUNTY_FIPS: &str = "55079";
pub const DETECTION_PRINT_LIMIT: usize = 50;
pub const LIQUOR_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

pub const LIQUOR_CANNOT_TELL: &str = "The file lists currently active licences and is overwritten nightly. It carries no licence number and no new-versus-renewed flag. A first-time licence and an annual renewal both appear as a row with a fresh effective date. A row appearing today and absent yesterday is new to the file. It is usually a new licence and may be a record correction. The first snapshot establishes a baseline and produces no signal.";

#[derive(Debug, Clone)]
pub struct LiquorFetch {
    pub status: u16,
    pub resolved_url: String,
    pub first_200: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquorRow {
    pub corp_name: Option<String>,
    pub trade_name: Option<String>,
    pub licensee: Option<String>,
    pub licensee_suffix: Option<String>,
    pub address: Option<String>,
    pub house_nr: Option<String>,
    pub sdir: Option<String>,
    pub street: Option<String>,
    pub sttype: Option<String>,
    pub aldermanic_district: Option<String>,
    pub police_district: Option<String>,
    pub lic_type: Option<String>,
    pub licence_type_full: Option<String>,
    pub tot_cap: Option<String>,
    pub room_cap: Option<String>,
    pub taxkey: Option<String>,
    pub eff_date: Option<String>,
    pub exp_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiquorLoadResult {
    pub snapshot_date: String,
    pub load_run_id: i64,
    pub row_count: i64,
    pub placeholder_rows: i64,
    pub already_loaded: bool,
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiquorSnapshotMeta {
    pub snapshot_date: String,
    pub row_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MilwaukeeParcelHit {
    pub owner_name: Option<String>,
    pub assessed_value: Option<f64>,
}

pub fn fetch_liquor_csv() -> Result<LiquorFetch, StoreError> {
    match ureq::get(LIQUOR_CSV_URL)
        .set("User-Agent", LIQUOR_USER_AGENT)
        .timeout(Duration::from_secs(60))
        .call()
    {
        Ok(response) => {
            let status = response.status();
            let resolved_url = response.get_url().to_string();
            let mut body = Vec::new();
            response
                .into_reader()
                .read_to_end(&mut body)
                .map_err(|err| StoreError::Liquor(err.to_string()))?;
            let n = body.len().min(200);
            let first_200 = decode_csv_bytes(&body[..n]);
            Ok(LiquorFetch {
                status,
                resolved_url,
                first_200,
                body,
            })
        }
        Err(ureq::Error::Status(status, response)) => {
            let resolved_url = response.get_url().to_string();
            let mut body = Vec::new();
            let _ = response.into_reader().read_to_end(&mut body);
            let n = body.len().min(200);
            let first_200 = decode_csv_bytes(&body[..n]);
            Err(StoreError::Liquor(format!(
                "status={status} resolved={resolved_url} first_200={first_200}"
            )))
        }
        Err(err) => Err(StoreError::Liquor(err.to_string())),
    }
}

pub fn decode_csv_bytes(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    bytes.iter().map(|&b| cp1252_char(b)).collect()
}

fn cp1252_char(b: u8) -> char {
    if b < 0x80 {
        return b as char;
    }
    match b {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        _ => b as char,
    }
}

pub fn parse_liquor_csv(text: &str) -> Result<Vec<LiquorRow>, StoreError> {
    let records = parse_csv(text)?;
    if records.is_empty() {
        return Err(StoreError::Liquor("liquor csv has no header".to_string()));
    }
    let header: Vec<String> = records[0].iter().map(|h| h.trim().to_string()).collect();
    let mut rows = Vec::new();
    for record in records.iter().skip(1) {
        if record.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let get = |name: &str| -> Option<String> {
            header
                .iter()
                .position(|h| h == name)
                .and_then(|i| record.get(i))
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let taxkey = get("TAXKEY").or_else(|| get("TAXKEY_NUMBER"));
        let published = get("LICENSEE");
        let (licensee, licensee_suffix) = split_licensee(published.as_deref());
        let house_nr = get("HOUSE_NR");
        let sdir = get("SDIR");
        let street = get("STREET");
        let sttype = get("STTYPE");
        let address = published_address(
            house_nr.as_deref(),
            sdir.as_deref(),
            street.as_deref(),
            sttype.as_deref(),
        );
        rows.push(LiquorRow {
            corp_name: get("CORP_NAME"),
            trade_name: get("TRADE_NAME"),
            licensee,
            licensee_suffix,
            address,
            house_nr,
            sdir,
            street,
            sttype,
            aldermanic_district: get("ALDERMANIC_DISTRICT"),
            police_district: get("POLICE_DISTRICT"),
            lic_type: get("LIC_TYPE"),
            licence_type_full: get("License Type Full Name"),
            tot_cap: get("TOT_CAP"),
            room_cap: get("ROOM_CAP"),
            taxkey,
            eff_date: get("EFF_DATE"),
            exp_date: get("EXP_DATE"),
        });
    }
    if rows.is_empty() {
        return Err(StoreError::Liquor(
            "liquor csv has a header and no licence rows".to_string(),
        ));
    }
    Ok(rows)
}

fn split_licensee(published: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = published.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    for suffix in ["Agt", "SP"] {
        let needle = format!(", {suffix}");
        if raw.ends_with(&needle) {
            return (Some(raw.to_string()), Some(suffix.to_string()));
        }
    }
    (Some(raw.to_string()), None)
}

fn published_address(
    house_nr: Option<&str>,
    sdir: Option<&str>,
    street: Option<&str>,
    sttype: Option<&str>,
) -> Option<String> {
    let parts: Vec<&str> = [house_nr, sdir, street, sttype]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, StoreError> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' => in_quotes = true,
            ',' => {
                record.push(std::mem::take(&mut field));
            }
            '\r' | '\n' => {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut field));
                if record.iter().any(|cell| !cell.is_empty()) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            other => field.push(other),
        }
    }
    if in_quotes {
        return Err(StoreError::Liquor("unterminated quote in liquor csv".to_string()));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(std::mem::take(&mut field));
        if record.iter().any(|cell| !cell.is_empty()) {
            records.push(record);
        }
    }
    Ok(records)
}

pub fn placeholder_row_count(rows: &[LiquorRow]) -> i64 {
    rows.iter()
        .filter(|row| row.taxkey.as_deref() == Some(PLACEHOLDER_TAXKEY))
        .count() as i64
}

pub fn load_liquor_snapshot(
    conn: &mut Connection,
    rows: &[LiquorRow],
    snapshot_date: &str,
    source_file: &str,
    fetched_at: &str,
) -> Result<LiquorLoadResult, StoreError> {
    if rows.is_empty() {
        return Err(StoreError::Liquor(
            "refusing to store an empty liquor snapshot".to_string(),
        ));
    }
    let placeholder_rows = placeholder_row_count(rows);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT load_run_id FROM liquor_snapshots WHERE snapshot_date = ?1",
            [snapshot_date],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(load_run_id) = existing {
        return Ok(LiquorLoadResult {
            snapshot_date: snapshot_date.to_string(),
            load_run_id,
            row_count: rows.len() as i64,
            placeholder_rows,
            already_loaded: true,
            source_file: source_file.to_string(),
        });
    }
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO load_runs (source_id, started_at, finished_at, status, records_requested, records_written)
         VALUES (?1, ?2, ?2, 'success', ?3, ?3)",
        params![LIQUOR_SOURCE_ID, fetched_at, rows.len() as i64],
    )?;
    let load_run_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO liquor_snapshots (snapshot_date, source_file, load_run_id, row_count, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            snapshot_date,
            source_file,
            load_run_id,
            rows.len() as i64,
            fetched_at
        ],
    )?;
    for (index, row) in rows.iter().enumerate() {
        tx.execute(
            "INSERT INTO liquor_licences (
                 snapshot_date, row_index, corp_name, trade_name, licensee, licensee_suffix,
                 address, house_nr, sdir, street, sttype, aldermanic_district, police_district,
                 lic_type, licence_type_full, tot_cap, room_cap, taxkey, eff_date, exp_date
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                snapshot_date,
                index as i64,
                row.corp_name,
                row.trade_name,
                row.licensee,
                row.licensee_suffix,
                row.address,
                row.house_nr,
                row.sdir,
                row.street,
                row.sttype,
                row.aldermanic_district,
                row.police_district,
                row.lic_type,
                row.licence_type_full,
                row.tot_cap,
                row.room_cap,
                row.taxkey,
                row.eff_date,
                row.exp_date
            ],
        )?;
    }
    tx.commit()?;
    Ok(LiquorLoadResult {
        snapshot_date: snapshot_date.to_string(),
        load_run_id,
        row_count: rows.len() as i64,
        placeholder_rows,
        already_loaded: false,
        source_file: source_file.to_string(),
    })
}

pub fn snapshot_date_for_load(existing: &[String], today: &str, fetched_at: &str) -> String {
    if existing.iter().any(|date| date == today) {
        fetched_at.to_string()
    } else {
        today.to_string()
    }
}

pub fn fetch_and_load_liquor(conn: &mut Connection) -> Result<(LiquorFetch, LiquorLoadResult), StoreError> {
    let fetch = fetch_liquor_csv()?;
    let text = decode_csv_bytes(&fetch.body);
    let rows = parse_liquor_csv(&text)?;
    let today: String = conn.query_row("SELECT date('now')", [], |row| row.get(0))?;
    let fetched_at: String =
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
            row.get(0)
        })?;
    let existing = snapshot_dates(conn)?;
    let snapshot_date = snapshot_date_for_load(&existing, &today, &fetched_at);
    let loaded = load_liquor_snapshot(
        conn,
        &rows,
        &snapshot_date,
        LIQUOR_CSV_URL,
        &fetched_at,
    )?;
    Ok((fetch, loaded))
}

pub fn snapshot_dates(conn: &Connection) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT snapshot_date FROM liquor_snapshots ORDER BY snapshot_date",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub type LicenceKey = (String, String);

pub fn licence_index(
    conn: &Connection,
    snapshot_date: &str,
) -> Result<(BTreeMap<LicenceKey, BTreeSet<String>>, i64), StoreError> {
    let mut stmt = conn.prepare(
        "SELECT taxkey, lic_type, licensee FROM liquor_licences WHERE snapshot_date = ?1",
    )?;
    let rows = stmt.query_map([snapshot_date], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut index: BTreeMap<LicenceKey, BTreeSet<String>> = BTreeMap::new();
    let mut placeholders = 0i64;
    for row in rows {
        let (taxkey, lic_type, licensee) = row?;
        let taxkey = taxkey.unwrap_or_default();
        if taxkey == PLACEHOLDER_TAXKEY {
            placeholders += 1;
            continue;
        }
        let lic_type = lic_type.unwrap_or_default();
        if taxkey.is_empty() || lic_type.is_empty() {
            continue;
        }
        index
            .entry((taxkey, lic_type))
            .or_default()
            .insert(licensee.unwrap_or_default());
    }
    Ok((index, placeholders))
}

pub fn snapshot_metas(conn: &Connection) -> Result<Vec<LiquorSnapshotMeta>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT snapshot_date, row_count FROM liquor_snapshots ORDER BY snapshot_date",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(LiquorSnapshotMeta {
            snapshot_date: row.get(0)?,
            row_count: row.get(1)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn liquor_rows(conn: &Connection, snapshot_date: &str) -> Result<Vec<LiquorRow>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT corp_name, trade_name, licensee, licensee_suffix, address, house_nr, sdir,
                street, sttype, aldermanic_district, police_district, lic_type, licence_type_full,
                tot_cap, room_cap, taxkey, eff_date, exp_date
         FROM liquor_licences
         WHERE snapshot_date = ?1
         ORDER BY row_index",
    )?;
    let rows = stmt.query_map([snapshot_date], |row| {
        Ok(LiquorRow {
            corp_name: row.get(0)?,
            trade_name: row.get(1)?,
            licensee: row.get(2)?,
            licensee_suffix: row.get(3)?,
            address: row.get(4)?,
            house_nr: row.get(5)?,
            sdir: row.get(6)?,
            street: row.get(7)?,
            sttype: row.get(8)?,
            aldermanic_district: row.get(9)?,
            police_district: row.get(10)?,
            lic_type: row.get(11)?,
            licence_type_full: row.get(12)?,
            tot_cap: row.get(13)?,
            room_cap: row.get(14)?,
            taxkey: row.get(15)?,
            eff_date: row.get(16)?,
            exp_date: row.get(17)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn liquor_rows_for_keys(
    conn: &Connection,
    snapshot_date: &str,
    keys: &[LicenceKey],
) -> Result<Vec<LiquorRow>, StoreError> {
    let wanted: BTreeSet<LicenceKey> = keys.iter().cloned().collect();
    Ok(liquor_rows(conn, snapshot_date)?
        .into_iter()
        .filter(|row| {
            let taxkey = row.taxkey.clone().unwrap_or_default();
            let lic_type = row.lic_type.clone().unwrap_or_default();
            wanted.contains(&(taxkey, lic_type))
        })
        .collect())
}

pub fn snapshots_every_row_matches(a: &[LiquorRow], b: &[LiquorRow]) -> bool {
    let mut left = a.to_vec();
    let mut right = b.to_vec();
    left.sort_by_key(row_sort_key);
    right.sort_by_key(row_sort_key);
    left == right
}

fn row_sort_key(row: &LiquorRow) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        row.taxkey,
        row.lic_type,
        row.licensee,
        row.address,
        row.corp_name,
        row.trade_name,
        row.eff_date,
        row.licence_type_full
    )
}

pub fn milwaukee_parcels_for_taxkey(
    conn: &Connection,
    taxkey: &str,
) -> Result<Vec<MilwaukeeParcelHit>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT owner_name, assessed_value
         FROM current_parcels
         WHERE county_fips = ?1 AND taxkey = ?2
         ORDER BY muni_code",
    )?;
    let rows = stmt.query_map(params![MILWAUKEE_COUNTY_FIPS, taxkey], |row| {
        Ok(MilwaukeeParcelHit {
            owner_name: row.get(0)?,
            assessed_value: row.get(1)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_csv() -> &'static str {
        "\"EXP_DATE\",\"EFF_DATE\",\"CORP_NAME\",\"TRADE_NAME\",\"LICENSEE\",\"TAXKEY_NUMBER\",\"HOUSE_NR\",\"SDIR\",\"STREET\",\"STTYPE\",\"ALDERMANIC_DISTRICT\",\"POLICE_DISTRICT\",\"LIC_TYPE\",\"License Type Full Name\",\"TOT_CAP\",\"ROOM_CAP\",\"TAXKEY\"\n\
\"2026-10-22\",\"2025-10-15\",\"AARYAN, LLC\",\"HAMPTON\",\"ALPESH B PATEL, Agt\",\"0400039100\",\"8608\",\"N\",\"107TH\",\"ST\",\"9\",\"4\",\"ALQML\",\"Class A Malt\",\"\",\"\",\"0400039100\"\n"
    }

    #[test]
    fn parse_splits_agent_suffix_and_builds_address() {
        let rows = parse_liquor_csv(fixture_csv()).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].taxkey.as_deref(), Some("0400039100"));
        assert_eq!(rows[0].licensee.as_deref(), Some("ALPESH B PATEL, Agt"));
        assert_eq!(rows[0].licensee_suffix.as_deref(), Some("Agt"));
        assert_eq!(rows[0].address.as_deref(), Some("8608 N 107TH ST"));
        assert_eq!(rows[0].lic_type.as_deref(), Some("ALQML"));
    }

    #[test]
    fn parse_accepts_carriage_return_record_separators() {
        let text = fixture_csv().replace('\n', "\r");
        let rows = parse_liquor_csv(&text).expect("parse cr");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].taxkey.as_deref(), Some("0400039100"));
    }

    #[test]
    fn snapshot_date_uses_fetched_at_when_today_is_already_stored() {
        let date = snapshot_date_for_load(
            &["2026-09-08".to_string()],
            "2026-09-08",
            "2026-09-08T16:15:00Z",
        );
        assert_eq!(date, "2026-09-08T16:15:00Z");
        let next_day = snapshot_date_for_load(
            &["2026-09-08".to_string()],
            "2026-09-09",
            "2026-09-09T00:01:00Z",
        );
        assert_eq!(next_day, "2026-09-09");
    }
}
