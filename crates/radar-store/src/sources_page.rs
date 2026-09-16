// Sources page sections are fixed: one card group per status, one
// record per sources.toml entry. This is not a page builder. Fetching
// is not offered here; it lives on the page that owns the data.

use radar_config::{RadarConfig, SourceKind, SourceStatus};
use rusqlite::Connection;
use serde::Serialize;

use crate::assignments::{
    USPTO_ASSIGNMENTS_FOLDER, USPTO_ASSIGNMENTS_SOURCE_ID, USPTO_DOWNLOAD_PAGE,
};
use crate::series::{days_between, today_ymd};
use crate::signals::{detection_built_status, DETECTION_BUILT};
use crate::StoreError;

const NEVER_FETCHED: &str = "never fetched";
const NO_CADENCE: &str = "no cadence; cannot be overdue";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceRow {
    pub source_id: String,
    pub display_name: String,
    pub family: String,
    pub kind: String,
    pub status: String,
    pub cadence_days: Option<u32>,
    pub publication_lag_days: Option<u32>,
    pub lag_measured: bool,
    pub url_verified: bool,
    pub requires_key: bool,
    pub attribution: Option<String>,
    pub notes: Option<String>,
    pub last_load_run_id: Option<i64>,
    pub last_finished_at: Option<String>,
    pub last_records_written: Option<i64>,
    pub days_ago: Option<i64>,
    pub fetch_text: String,
    pub overdue: bool,
    pub overdue_text: String,
    pub folder_path: Option<String>,
    pub download_page: Option<String>,
    pub signal_count: i64,
    pub detection_built_count: i64,
    pub live_without_detection: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceGroup {
    pub status: String,
    pub rows: Vec<SourceRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourcesPage {
    pub vertical_display_name: String,
    pub county_name: String,
    pub county_fips: String,
    pub config_version: String,
    pub groups: Vec<SourceGroup>,
}

pub fn list_sources(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<SourcesPage, StoreError> {
    let today = today_ymd();
    let mut indexed: Vec<(usize, SourceRow)> = Vec::new();
    for (index, source) in config.sources.sources.iter().enumerate() {
        indexed.push((index, source_row(conn, source, &today)?));
    }
    indexed.sort_by(|a, b| {
        status_rank(&a.1.status)
            .cmp(&status_rank(&b.1.status))
            .then(a.0.cmp(&b.0))
    });
    let mut groups = Vec::new();
    for status in ["Live", "Profiled", "Scoped", "Refused"] {
        let rows: Vec<SourceRow> = indexed
            .iter()
            .filter(|(_, row)| row.status == status)
            .map(|(_, row)| row.clone())
            .collect();
        if !rows.is_empty() {
            groups.push(SourceGroup {
                status: status.to_string(),
                rows,
            });
        }
    }
    Ok(SourcesPage {
        vertical_display_name: config.manifest.vertical_display_name.clone(),
        county_name: config.manifest.county_name.clone(),
        county_fips: config.manifest.county_fips.clone(),
        config_version: config.manifest.config_version.clone(),
        groups,
    })
}

fn status_rank(status: &str) -> u8 {
    match status {
        "Live" => 0,
        "Profiled" => 1,
        "Scoped" => 2,
        "Refused" => 3,
        _ => 4,
    }
}

fn source_row(
    conn: &Connection,
    source: &radar_config::SourceEntry,
    today: &str,
) -> Result<SourceRow, StoreError> {
    let last = latest_success(conn, &source.source_id)?;
    let last_finished_at = last.as_ref().and_then(|run| run.finished_at.clone());
    let last_records_written = last.as_ref().and_then(|run| run.records_written);
    let last_load_run_id = last.as_ref().map(|run| run.id);
    let days_ago = last_finished_at
        .as_deref()
        .and_then(|finished| days_between(finished, today));
    let fetch_text = match last.as_ref() {
        None => NEVER_FETCHED.to_string(),
        Some(run) => {
            let date = run
                .finished_at
                .as_deref()
                .map(date_prefix)
                .unwrap_or("unknown date");
            let count = match run.records_written {
                Some(n) => format!("{n} records"),
                None => "record count unknown".to_string(),
            };
            match days_ago {
                Some(days) => format!("{count} on {date}, {days} days ago"),
                None => format!("{count} on {date}"),
            }
        }
    };
    let (overdue, overdue_text) = overdue_state(source.cadence_days, days_ago, last.is_some());
    let (folder_path, download_page) = manual_paths(source);
    let (signal_count, detection_built_count) = signal_counts(conn, &source.source_id)?;
    let live_without_detection =
        source.status == SourceStatus::Live && detection_built_count == 0;
    Ok(SourceRow {
        source_id: source.source_id.clone(),
        display_name: source.display_name.clone(),
        family: source.family.as_label().to_string(),
        kind: source.kind.as_label().to_string(),
        status: source.status.as_label().to_string(),
        cadence_days: source.cadence_days,
        publication_lag_days: source.publication_lag_days,
        lag_measured: source.lag_measured,
        url_verified: source.url_verified,
        requires_key: source.requires_key,
        attribution: source.attribution.clone(),
        notes: source.notes.clone(),
        last_load_run_id,
        last_finished_at,
        last_records_written,
        days_ago,
        fetch_text,
        overdue,
        overdue_text,
        folder_path,
        download_page,
        signal_count,
        detection_built_count,
        live_without_detection,
    })
}

struct LastRun {
    id: i64,
    finished_at: Option<String>,
    records_written: Option<i64>,
}

fn latest_success(conn: &Connection, source_id: &str) -> Result<Option<LastRun>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, finished_at, records_written
         FROM load_runs
         WHERE source_id = ?1 AND status = 'success'
         ORDER BY finished_at DESC, id DESC
         LIMIT 1",
    )?;
    let mut rows = stmt.query([source_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(LastRun {
            id: row.get(0)?,
            finished_at: row.get(1)?,
            records_written: row.get(2)?,
        })),
        None => Ok(None),
    }
}

fn overdue_state(
    cadence_days: Option<u32>,
    days_ago: Option<i64>,
    fetched: bool,
) -> (bool, String) {
    let Some(cadence) = cadence_days else {
        return (false, NO_CADENCE.to_string());
    };
    if !fetched {
        return (false, NEVER_FETCHED.to_string());
    }
    match days_ago {
        Some(days) if days > cadence as i64 => (
            true,
            format!("overdue, {days} days since last successful fetch (cadence {cadence} days)"),
        ),
        Some(days) => (
            false,
            format!("{days} days since last successful fetch (cadence {cadence} days)"),
        ),
        None => (false, "last fetch date could not be parsed".to_string()),
    }
}

fn manual_paths(source: &radar_config::SourceEntry) -> (Option<String>, Option<String>) {
    if source.kind != SourceKind::ManualUpload {
        return (None, None);
    }
    let folder_path = if source.source_id == USPTO_ASSIGNMENTS_SOURCE_ID {
        Some(USPTO_ASSIGNMENTS_FOLDER.to_string())
    } else {
        None
    };
    let download_page = source
        .url
        .clone()
        .or_else(|| source.notes.as_deref().and_then(url_from_notes))
        .or_else(|| {
            if source.source_id == USPTO_ASSIGNMENTS_SOURCE_ID {
                Some(USPTO_DOWNLOAD_PAGE.to_string())
            } else {
                None
            }
        });
    (folder_path, download_page)
}

fn url_from_notes(notes: &str) -> Option<String> {
    for token in notes.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| {
            matches!(c, '.' | ',' | ';' | ')' | '(' | '"' | '\'')
        });
        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn signal_counts(conn: &Connection, source_id: &str) -> Result<(i64, i64), StoreError> {
    let mut stmt = conn.prepare(
        "SELECT signal_key, detection_method FROM signal_catalog WHERE source_id = ?1",
    )?;
    let rows = stmt.query_map([source_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut signal_count = 0i64;
    let mut detection_built_count = 0i64;
    for row in rows {
        let (signal_key, method) = row?;
        signal_count += 1;
        if detection_built_status(&signal_key, &method) == DETECTION_BUILT {
            detection_built_count += 1;
        }
    }
    Ok((signal_count, detection_built_count))
}

fn date_prefix(value: &str) -> &str {
    if value.len() >= 10 {
        &value[0..10]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rank::tests::{test_bands, test_config};
    use crate::series::add_days;
    use radar_config::load::load;
    use radar_config::sources::{SourceEntry, SourceFamily, SourceKind, SourceStatus};
    use rusqlite::params;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        crate::migrate::migrate(&mut conn).expect("migrate");
        conn
    }

    fn source(
        id: &str,
        status: SourceStatus,
        kind: SourceKind,
        cadence: Option<u32>,
        notes: Option<&str>,
        url: Option<&str>,
    ) -> SourceEntry {
        SourceEntry {
            source_id: id.to_string(),
            display_name: id.to_string(),
            family: SourceFamily::LegalPublicRecord,
            kind,
            status,
            cadence_days: cadence,
            publication_lag_days: None,
            lag_measured: false,
            url: url.map(str::to_string),
            url_verified: url.is_some(),
            requires_key: false,
            attribution: None,
            notes: notes.map(str::to_string),
        }
    }

    fn insert_run(
        conn: &Connection,
        source_id: &str,
        finished_at: &str,
        records_written: Option<i64>,
        status: &str,
    ) {
        conn.execute(
            "INSERT INTO load_runs (source_id, status, finished_at, records_written)
             VALUES (?1, ?2, ?3, ?4)",
            params![source_id, status, finished_at, records_written],
        )
        .expect("run");
    }

    fn page_rows(page: &SourcesPage) -> Vec<&SourceRow> {
        page.groups.iter().flat_map(|group| group.rows.iter()).collect()
    }

    fn find<'a>(page: &'a SourcesPage, id: &str) -> &'a SourceRow {
        page_rows(page)
            .into_iter()
            .find(|row| row.source_id == id)
            .expect("source")
    }

    #[test]
    fn every_source_in_loaded_config_appears_exactly_once() {
        let conn = open();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/dr_data_icp");
        let config = load(&dir).expect("load dr_data_icp");
        let page = list_sources(&config, &conn).expect("page");
        let ids: Vec<&str> = page_rows(&page)
            .iter()
            .map(|row| row.source_id.as_str())
            .collect();
        let unique: HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), config.sources.sources.len());
        assert_eq!(unique.len(), ids.len());
        for source in &config.sources.sources {
            assert!(
                unique.contains(source.source_id.as_str()),
                "missing {}",
                source.source_id
            );
        }
    }

    #[test]
    fn source_with_no_load_run_shows_never_fetched_not_zero() {
        let conn = open();
        let mut config = test_config(1, "caveat", test_bands());
        config.sources.sources = vec![source(
            "src-none",
            SourceStatus::Live,
            SourceKind::Csv,
            Some(7),
            None,
            None,
        )];
        let page = list_sources(&config, &conn).expect("page");
        let row = find(&page, "src-none");
        assert_eq!(row.fetch_text, NEVER_FETCHED);
        assert!(row.last_load_run_id.is_none());
        assert!(row.last_records_written.is_none());
        assert_ne!(row.fetch_text, "0");
        assert_ne!(row.last_records_written, Some(0));
    }

    #[test]
    fn source_past_cadence_shows_overdue_with_day_count() {
        let conn = open();
        let finished = add_days(&today_ymd(), -40).expect("date");
        insert_run(&conn, "src-stale", &finished, Some(12), "success");
        let mut config = test_config(1, "caveat", test_bands());
        config.sources.sources = vec![source(
            "src-stale",
            SourceStatus::Live,
            SourceKind::Csv,
            Some(7),
            None,
            None,
        )];
        let page = list_sources(&config, &conn).expect("page");
        let row = find(&page, "src-stale");
        assert!(row.overdue);
        assert_eq!(row.days_ago, Some(40));
        assert!(row.overdue_text.contains("overdue"));
        assert!(row.overdue_text.contains("40"));
        assert_eq!(row.last_records_written, Some(12));
        assert!(row.fetch_text.contains("12 records"));
    }

    #[test]
    fn source_with_no_cadence_is_not_marked_overdue() {
        let conn = open();
        let finished = add_days(&today_ymd(), -400).expect("date");
        insert_run(&conn, "src-open", &finished, Some(3), "success");
        let mut config = test_config(1, "caveat", test_bands());
        config.sources.sources = vec![source(
            "src-open",
            SourceStatus::Live,
            SourceKind::Api,
            None,
            None,
            None,
        )];
        let page = list_sources(&config, &conn).expect("page");
        let row = find(&page, "src-open");
        assert!(!row.overdue);
        assert_eq!(row.overdue_text, NO_CADENCE);
        assert_eq!(row.days_ago, Some(400));
    }

    #[test]
    fn manual_upload_source_shows_folder_and_download_url() {
        let conn = open();
        let mut config = test_config(1, "caveat", test_bands());
        config.sources.sources = vec![source(
            USPTO_ASSIGNMENTS_SOURCE_ID,
            SourceStatus::Live,
            SourceKind::ManualUpload,
            Some(7),
            Some("Download daily zip files from https://data.uspto.gov/bulkdata/datasets/pasdl by hand."),
            None,
        )];
        let page = list_sources(&config, &conn).expect("page");
        let row = find(&page, USPTO_ASSIGNMENTS_SOURCE_ID);
        assert_eq!(row.kind, "ManualUpload");
        assert_eq!(
            row.folder_path.as_deref(),
            Some(USPTO_ASSIGNMENTS_FOLDER)
        );
        assert_eq!(
            row.download_page.as_deref(),
            Some("https://data.uspto.gov/bulkdata/datasets/pasdl")
        );
    }

    #[test]
    fn refused_sources_appear_last_with_their_reason() {
        let conn = open();
        let mut config = test_config(1, "caveat", test_bands());
        config.sources.sources = vec![
            source(
                "src-refused",
                SourceStatus::Refused,
                SourceKind::ManualUpload,
                None,
                Some("No bulk open data route exists."),
                None,
            ),
            source(
                "src-live",
                SourceStatus::Live,
                SourceKind::Csv,
                Some(1),
                None,
                None,
            ),
            source(
                "src-scoped",
                SourceStatus::Scoped,
                SourceKind::Api,
                None,
                None,
                None,
            ),
            source(
                "src-profiled",
                SourceStatus::Profiled,
                SourceKind::Csv,
                Some(30),
                None,
                None,
            ),
        ];
        let page = list_sources(&config, &conn).expect("page");
        let statuses: Vec<&str> = page.groups.iter().map(|group| group.status.as_str()).collect();
        assert_eq!(statuses, vec!["Live", "Profiled", "Scoped", "Refused"]);
        let ids: Vec<&str> = page_rows(&page)
            .iter()
            .map(|row| row.source_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["src-live", "src-profiled", "src-scoped", "src-refused"]
        );
        let refused = find(&page, "src-refused");
        assert_eq!(
            refused.notes.as_deref(),
            Some("No bulk open data route exists.")
        );
    }

    #[test]
    fn signal_count_matches_the_catalogue() {
        let conn = open();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/dr_data_icp");
        let config = load(&dir).expect("load");
        let page = list_sources(&config, &conn).expect("page");
        let mut stmt = conn
            .prepare("SELECT signal_key, detection_method FROM signal_catalog WHERE source_id = ?1")
            .expect("stmt");
        for source in &config.sources.sources {
            let catalog: Vec<(String, String)> = stmt
                .query_map([&source.source_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("rows")
                .map(|row| row.expect("row"))
                .collect();
            let expected_built = catalog
                .iter()
                .filter(|(key, method)| detection_built_status(key, method) == DETECTION_BUILT)
                .count() as i64;
            let row = find(&page, &source.source_id);
            assert_eq!(
                row.signal_count,
                catalog.len() as i64,
                "{}",
                source.source_id
            );
            assert_eq!(
                row.detection_built_count, expected_built,
                "{}",
                source.source_id
            );
            if source.status == SourceStatus::Live && expected_built == 0 {
                assert!(row.live_without_detection, "{}", source.source_id);
            }
        }
    }
}
