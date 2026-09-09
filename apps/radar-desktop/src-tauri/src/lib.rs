use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use radar_config::{nav_pages, page_is_enabled, NavPage, PAGE_NOT_ENABLED, SessionState};
use radar_store::{
    assignment_detail as read_assignment_detail, assignment_filter_options as read_assignment_filter_options,
    assignment_folder_status as read_assignment_folder_status, export_assignments as write_assignment_export,
    export_owner_groups as write_owner_group_export, insert_outcome as write_outcome,
    list_assignments as read_assignments, list_detections as read_detections, list_filter_options,
    list_group_parcels, list_market_watch as read_market_watch, list_coverage as read_coverage_page,
    list_sources as read_sources_page, list_owner_groups as read_owner_groups,
    migrate, outcome_history as read_outcome_history, read_glossary,
    refresh_assignments as run_assignment_refresh, signal_catalogue as read_signal_catalogue,
    AssignmentDetail, AssignmentExportMeta, AssignmentExportResult, AssignmentFilterOptions,
    AssignmentFolderStatus, AssignmentListPage, AssignmentListQuery, AssignmentRefreshResult,
    DetectionListPage, DetectionListQuery, GlossaryRead, GroupExportResult, GroupFilterOptions,
    GroupListPage, GroupListQuery, GroupParcelRow, MarketWatchPage, CoveragePage, SourcesPage, OutcomeHistory, OutcomeRecord,
    RecordOutcomeRequest, SignalCataloguePage, SignalCatalogueQuery, USPTO_ASSIGNMENTS_FOLDER,
};
use rusqlite::Connection;
use serde::Serialize;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn config_dir() -> PathBuf {
    match env::var("RADAR_CONFIG_DIR") {
        Ok(value) if !value.trim().is_empty() => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repo_root().join(path)
            }
        }
        _ => repo_root().join("config/dr_data_icp"),
    }
}

fn open_store() -> Result<Connection, String> {
    let db_path = repo_root().join("data/radar.db");
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut conn = Connection::open(&db_path).map_err(|err| err.to_string())?;
    migrate(&mut conn).map_err(|err| err.to_string())?;
    Ok(conn)
}

fn load_ready_config() -> Result<radar_config::RadarConfig, String> {
    let dir = config_dir();
    match SessionState::from_dir(&dir).map_err(|err| err.to_string())? {
        SessionState::Ready { config, .. } => Ok(config),
        SessionState::Unconfigured => Err(format!(
            "config directory is unconfigured: {}",
            dir.display()
        )),
        SessionState::Invalid { path, findings } => {
            let details: Vec<String> = findings
                .into_iter()
                .map(|finding| format!("{}: {}", finding.location, finding.message))
                .collect();
            Err(format!(
                "config at {} is invalid: {}",
                path.display(),
                details.join("; ")
            ))
        }
    }
}

fn require_page(page_key: &str) -> Result<radar_config::RadarConfig, String> {
    let config = load_ready_config()?;
    if page_is_enabled(&config.manifest, page_key) {
        Ok(config)
    } else {
        Err(PAGE_NOT_ENABLED.to_string())
    }
}

#[derive(Debug, Serialize)]
struct PageNav {
    page_key: String,
    enabled: bool,
    disabled_text: Option<String>,
    pages: Vec<NavPage>,
}

#[tauri::command]
fn page_nav(page_key: String) -> Result<PageNav, String> {
    let config = load_ready_config()?;
    let enabled = page_is_enabled(&config.manifest, &page_key);
    Ok(PageNav {
        page_key: page_key.clone(),
        enabled,
        disabled_text: if enabled {
            None
        } else {
            Some(PAGE_NOT_ENABLED.to_string())
        },
        pages: nav_pages(&config.manifest),
    })
}

fn config_fingerprint(config_dir: &Path) -> String {
    radar_config::config_fingerprint(config_dir)
}

#[tauri::command]
fn glossary(key: Option<String>) -> Result<GlossaryRead, String> {
    let conn = open_store()?;
    read_glossary(&conn, key.as_deref()).map_err(|err| err.to_string())
}

#[derive(Debug, Serialize)]
struct StatusSnapshot {
    latest_load_run_id: Option<i64>,
    source_id: Option<String>,
    value_as_of: Option<String>,
    records_written: Option<i64>,
    config_fingerprint: String,
}

#[tauri::command]
fn status_snapshot() -> Result<StatusSnapshot, String> {
    let conn = open_store()?;
    let row = conn
        .query_row(
            "SELECT id, source_id, value_as_of, records_written
             FROM load_runs
             WHERE status = 'success'
             ORDER BY finished_at DESC, id DESC
             LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .ok();
    let fingerprint = config_fingerprint(&config_dir());
    Ok(match row {
        Some((id, source_id, value_as_of, records_written)) => StatusSnapshot {
            latest_load_run_id: Some(id),
            source_id: Some(source_id),
            value_as_of,
            records_written,
            config_fingerprint: fingerprint,
        },
        None => StatusSnapshot {
            latest_load_run_id: None,
            source_id: None,
            value_as_of: None,
            records_written: None,
            config_fingerprint: fingerprint,
        },
    })
}

#[tauri::command]
async fn owner_group_filter_options() -> Result<GroupFilterOptions, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let config = load_ready_config()?;
        let conn = open_store()?;
        list_filter_options(&config, &conn).map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn list_owner_groups(query: GroupListQuery) -> Result<GroupListPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = require_page("owner_groups")?;
        let conn = open_store()?;
        read_owner_groups(&config, &conn, &query).map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn owner_group_parcels(group_key: String) -> Result<Vec<GroupParcelRow>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = load_ready_config()?;
        let conn = open_store()?;
        list_group_parcels(&config, &conn, &group_key).map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn export_owner_groups(query: GroupListQuery) -> Result<GroupExportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = load_ready_config()?;
        let conn = open_store()?;
        let file_name = format!("{}_owner_groups.csv", config.manifest.vertical_id);
        let path = repo_root().join("exports").join(file_name);
        write_owner_group_export(&config, &conn, &query, &path).map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
fn signal_catalogue(query: SignalCatalogueQuery) -> Result<SignalCataloguePage, String> {
    let config = load_ready_config()?;
    let conn = open_store()?;
    read_signal_catalogue(&config, &conn, &query).map_err(|err| err.to_string())
}

#[tauri::command]
fn list_detections(query: DetectionListQuery) -> Result<DetectionListPage, String> {
    let config = load_ready_config()?;
    let conn = open_store()?;
    read_detections(&config, &conn, &query).map_err(|err| err.to_string())
}

#[tauri::command]
fn assignment_filter_options() -> Result<AssignmentFilterOptions, String> {
    let config = load_ready_config()?;
    let conn = open_store()?;
    read_assignment_filter_options(&config, &conn).map_err(|err| err.to_string())
}

#[tauri::command]
fn list_assignments(query: AssignmentListQuery) -> Result<AssignmentListPage, String> {
    let config = require_page("assignments")?;
    let conn = open_store()?;
    read_assignments(&config, &conn, &query).map_err(|err| err.to_string())
}

#[tauri::command]
fn assignment_detail(reel_no: String, frame_no: String) -> Result<AssignmentDetail, String> {
    let config = load_ready_config()?;
    let conn = open_store()?;
    read_assignment_detail(&config, &conn, &reel_no, &frame_no).map_err(|err| err.to_string())
}

#[tauri::command]
fn assignment_folder_status() -> Result<AssignmentFolderStatus, String> {
    let conn = open_store()?;
    read_assignment_folder_status(&conn, Path::new(USPTO_ASSIGNMENTS_FOLDER))
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn refresh_assignments() -> Result<AssignmentRefreshResult, String> {
    let config = load_ready_config()?;
    let mut conn = open_store()?;
    run_assignment_refresh(&mut conn, Path::new(USPTO_ASSIGNMENTS_FOLDER), &config.assignments)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn pick_assignment_export_path() -> Result<Option<String>, String> {
    let suggested = repo_root().join("exports").join("assignments.csv");
    if let Some(parent) = suggested.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.SaveFileDialog; $d.Filter = 'CSV (*.csv)|*.csv'; $d.FileName = 'assignments.csv'; $d.InitialDirectory = '{}'; $d.Title = 'Export filtered assignments'; if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ $d.FileName }}",
        suggested
            .parent()
            .unwrap_or(Path::new("."))
            .display()
            .to_string()
            .replace('\'', "''")
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", &script])
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(path))
    }
}

#[tauri::command]
fn market_watch() -> Result<MarketWatchPage, String> {
    let config = require_page("market_watch")?;
    let conn = open_store()?;
    read_market_watch(&config, &conn).map_err(|err| err.to_string())
}

#[tauri::command]
fn coverage_page() -> Result<CoveragePage, String> {
    let config = require_page("coverage")?;
    let conn = open_store()?;
    read_coverage_page(&config, &conn).map_err(|err| err.to_string())
}

#[tauri::command]
fn sources_page() -> Result<SourcesPage, String> {
    let config = require_page("sources")?;
    let conn = open_store()?;
    read_sources_page(&config, &conn).map_err(|err| err.to_string())
}

#[tauri::command]
fn record_outcome(mut request: RecordOutcomeRequest) -> Result<OutcomeRecord, String> {
    request.config_fingerprint = config_fingerprint(&config_dir());
    let conn = open_store()?;
    write_outcome(&conn, &request).map_err(|err| err.to_string())
}

#[tauri::command]
fn outcome_history(subject_type: String, subject_key: String) -> Result<OutcomeHistory, String> {
    let conn = open_store()?;
    read_outcome_history(&conn, &subject_type, &subject_key).map_err(|err| err.to_string())
}

#[tauri::command]
fn export_assignments(
    query: AssignmentListQuery,
    path: String,
) -> Result<AssignmentExportResult, String> {
    let config = load_ready_config()?;
    let conn = open_store()?;
    let dir = config_dir();
    let meta = AssignmentExportMeta {
        config_directory: dir.display().to_string(),
        config_fingerprint: config_fingerprint(&dir),
    };
    write_assignment_export(&config, &conn, &query, Path::new(&path), &meta)
        .map_err(|err| err.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            glossary,
            status_snapshot,
            page_nav,
            owner_group_filter_options,
            list_owner_groups,
            owner_group_parcels,
            export_owner_groups,
            signal_catalogue,
            list_detections,
            assignment_filter_options,
            list_assignments,
            assignment_detail,
            assignment_folder_status,
            refresh_assignments,
            pick_assignment_export_path,
            export_assignments,
            market_watch,
            coverage_page,
            sources_page,
            record_outcome,
            outcome_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod command_probe {
    use super::*;
    use radar_store::{explain_owner_group_list_plan, GroupListQuery};

    #[test]
    fn probe_owner_groups_page_commands() {
        let conn = open_store().expect("open store");
        let county = load_ready_config()
            .expect("config")
            .manifest
            .county_fips;
        let parcel_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1",
                [&county],
                |row| row.get(0),
            )
            .expect("parcel count");
        let named_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_parcels
                 WHERE county_fips = ?1 AND owner_name IS NOT NULL",
                [&county],
                |row| row.get(0),
            )
            .expect("named count");
        println!("MEASURE county_fips={county}");
        println!("MEASURE current_parcels county rows={parcel_count} with owner_name={named_count}");
        println!(
            "MEASURE before SQL rewrite, every named parcel was pulled into Rust and grouped there; the page then kept 100 rows"
        );
        let config = load_ready_config().expect("config");
        match explain_owner_group_list_plan(&config, &conn) {
            Ok(plan) => {
                for line in plan {
                    println!("MEASURE plan {line}");
                }
            }
            Err(err) => println!("MEASURE plan error {err}"),
        }

        for run in 1..=3 {
            let started = std::time::Instant::now();
            let result = list_filter_options(&config, &conn);
            let elapsed = started.elapsed();
            match result {
                Ok(options) => println!(
                    "MEASURE owner_group_filter_options run={run} wall_ms={} municipalities={} owner_types={} bands={}",
                    elapsed.as_millis(),
                    options.municipalities.len(),
                    options.owner_types.len(),
                    options.bands.len()
                ),
                Err(err) => println!(
                    "MEASURE owner_group_filter_options run={run} wall_ms={} error {err}",
                    elapsed.as_millis()
                ),
            }
        }

        let query = GroupListQuery {
            apply_default_owner_type_exclusions: Some(true),
            ..GroupListQuery::default()
        };
        for run in 1..=3 {
            let started = std::time::Instant::now();
            let result = read_owner_groups(&config, &conn, &query);
            let elapsed = started.elapsed();
            match result {
                Ok(page) => println!(
                    "MEASURE list_owner_groups run={run} wall_ms={} rows={} total_before_filter={} total_after_filter={}",
                    elapsed.as_millis(),
                    page.rows.len(),
                    page.total_before_filter,
                    page.total_after_filter
                ),
                Err(err) => println!(
                    "MEASURE list_owner_groups run={run} wall_ms={} error {err}",
                    elapsed.as_millis()
                ),
            }
        }
    }
}
