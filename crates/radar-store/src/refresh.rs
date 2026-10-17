//! Sequencer for the nine unattended source steps. Each step calls the
//! existing loader functions. It does not delete or overwrite prior loads.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use radar_config::SessionState;
use rusqlite::Connection;

use crate::accela::{
    detect_occupancy_new_business, detect_occupancy_new_business_for_load,
    fetch_and_load_accela_occupancy, newest_stored_record_date, next_poll_start_date,
    print_accela_load,
};
use crate::anthropic::{
    anthropic_api_key_from_env, anthropic_model_from_env, AnthropicClient, ANTHROPIC_API_KEY_ABSENT,
};
use crate::assignments::{
    list_assignment_zips, load_assignments_from_folder, prepare_assignments_folder,
    USPTO_ASSIGNMENTS_FOLDER, USPTO_DOWNLOAD_PAGE,
};
use crate::current::rebuild_current_parcels;
use crate::detection::liquor::{
    detect_liquor_latest, print_liquor_detections, print_liquor_diff, LiquorComparability,
};
use crate::lira::{list_lira_csvs, load_lira_from_folder, LIRA_APPLICATIONS_FOLDER};
use crate::lira_extract::extract_folder;
use crate::lira_pdf::{
    fetch_pdfs_from_url_file, list_lira_pdfs, print_pdf_fetch, LivePdfHttp, LIRA_PDF_FOLDER,
    LIRA_PDF_URLS_NAME,
};
use crate::liquor::fetch_and_load_liquor;
use crate::migrate::migrate;
use crate::parcels::{load_parcels, LiveParcelSource};
use crate::series::{
    fred_api_key_from_env, load_fred_series, LiveFredClient, FRED_API_KEY_ABSENT,
};
use crate::StoreError;

pub const REQUIRED_REPO: &str = r"C:\Users\zumah\DrData_ICP_Lead_radar";
const DEFAULT_DB_PATH: &str = "data/radar.db";
const DEFAULT_DOWNLOADS: &str = r"C:\Users\zumah\Downloads";
const ACCELA_SINCE_NEEDED: &str =
    "needs --since MM/DD/YYYY: no usable record_date, so the next poll start cannot be computed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepId {
    FetchParcelsWaukesha,
    FetchParcelsMilwaukee,
    RebuildCurrentParcels,
    FetchFred,
    LoadMilwaukeeLiquor,
    LoadAccelaOccupancy,
    LoadUsptoAssignments,
    LoadLiraApplications,
    FetchAndExtractLiraPdfs,
}

pub const SEQUENCE: [StepId; 9] = [
    StepId::FetchParcelsWaukesha,
    StepId::FetchParcelsMilwaukee,
    StepId::RebuildCurrentParcels,
    StepId::FetchFred,
    StepId::LoadMilwaukeeLiquor,
    StepId::LoadAccelaOccupancy,
    StepId::LoadUsptoAssignments,
    StepId::LoadLiraApplications,
    StepId::FetchAndExtractLiraPdfs,
];

impl StepId {
    pub fn source_name(self) -> &'static str {
        match self {
            StepId::FetchParcelsWaukesha => "waukesha_parcels",
            StepId::FetchParcelsMilwaukee => "milwaukee_parcels",
            StepId::RebuildCurrentParcels => "rebuild_current_parcels",
            StepId::FetchFred => "fetch_fred",
            StepId::LoadMilwaukeeLiquor => "milwaukee_liquor",
            StepId::LoadAccelaOccupancy => "accela_occupancy",
            StepId::LoadUsptoAssignments => "uspto_assignments",
            StepId::LoadLiraApplications => "lira_applications",
            StepId::FetchAndExtractLiraPdfs => "lira_pdfs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Succeeded,
    Failed,
    Skipped,
    NeedsManualFile,
    NeedsApiKey,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Succeeded => "succeeded",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
            StepStatus::NeedsManualFile => "needs manual file",
            StepStatus::NeedsApiKey => "needs API key",
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(self, StepStatus::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepOutcome {
    pub status: StepStatus,
    pub count: Option<i64>,
    pub reason: Option<String>,
}

impl StepOutcome {
    fn succeeded(count: i64) -> Self {
        Self {
            status: StepStatus::Succeeded,
            count: Some(count),
            reason: None,
        }
    }

    fn failed(reason: String) -> Self {
        Self {
            status: StepStatus::Failed,
            count: None,
            reason: Some(reason),
        }
    }

    fn needs_manual_file(reason: String) -> Self {
        Self {
            status: StepStatus::NeedsManualFile,
            count: None,
            reason: Some(reason),
        }
    }

    fn needs_api_key(reason: String) -> Self {
        Self {
            status: StepStatus::NeedsApiKey,
            count: None,
            reason: Some(reason),
        }
    }

    fn skipped(reason: String) -> Self {
        Self {
            status: StepStatus::Skipped,
            count: None,
            reason: Some(reason),
        }
    }

    pub fn count_or_reason(&self) -> String {
        if let Some(count) = self.count {
            if let Some(reason) = &self.reason {
                format!("{count}; {reason}")
            } else {
                count.to_string()
            }
        } else {
            self.reason.clone().unwrap_or_else(|| "-".to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    pub step: StepId,
    pub status: StepStatus,
    pub count_or_reason: String,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshReport {
    pub steps: Vec<StepReport>,
}

impl RefreshReport {
    pub fn succeeded_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.status == StepStatus::Succeeded)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.status.is_failure())
            .count()
    }

    pub fn skipped_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| {
                matches!(
                    s.status,
                    StepStatus::Skipped | StepStatus::NeedsManualFile | StepStatus::NeedsApiKey
                )
            })
            .count()
    }
}

#[derive(Debug, Clone)]
pub struct RefreshPaths {
    pub db_path: PathBuf,
    pub waukesha_config: PathBuf,
    pub milwaukee_config: PathBuf,
    pub assignments_folder: PathBuf,
    pub assignments_downloads: PathBuf,
    pub lira_csv_folder: PathBuf,
    pub lira_pdf_folder: PathBuf,
    pub rasterize_script: PathBuf,
}

impl RefreshPaths {
    pub fn live(repo: &Path) -> Self {
        Self {
            db_path: repo.join(DEFAULT_DB_PATH),
            waukesha_config: repo.join("config/dr_data_icp"),
            milwaukee_config: repo.join("config/milwaukee_parcels"),
            assignments_folder: PathBuf::from(USPTO_ASSIGNMENTS_FOLDER),
            assignments_downloads: PathBuf::from(DEFAULT_DOWNLOADS),
            lira_csv_folder: PathBuf::from(LIRA_APPLICATIONS_FOLDER),
            lira_pdf_folder: PathBuf::from(LIRA_PDF_FOLDER),
            rasterize_script: repo.join("scripts").join("rasterize_pdf.py"),
        }
    }

    pub fn urls_txt(&self) -> PathBuf {
        self.lira_pdf_folder.join(LIRA_PDF_URLS_NAME)
    }
}

#[derive(Debug, Clone)]
pub struct RefreshContext {
    pub paths: RefreshPaths,
    pub fred_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub accela_poll_start: Option<String>,
}

impl RefreshContext {
    pub fn for_live(repo: &Path) -> Result<Self, StoreError> {
        let paths = RefreshPaths::live(repo);
        let accela_poll_start = accela_poll_start_from_db(&paths.db_path)?;
        Ok(Self {
            paths,
            fred_api_key: fred_api_key_from_env(),
            anthropic_api_key: anthropic_api_key_from_env(),
            accela_poll_start,
        })
    }
}

pub fn accela_poll_start_from_db(db_path: &Path) -> Result<Option<String>, StoreError> {
    if !db_path.exists() {
        return Ok(None);
    }
    let conn = Connection::open(db_path)?;
    let newest = newest_stored_record_date(&conn)?;
    Ok(next_poll_start_date(newest.as_deref()))
}

pub trait StepExecutor {
    fn execute(&mut self, step: StepId, ctx: &RefreshContext) -> StepOutcome;
}

pub struct LiveExecutor;

impl StepExecutor for LiveExecutor {
    fn execute(&mut self, step: StepId, ctx: &RefreshContext) -> StepOutcome {
        match step {
            StepId::FetchParcelsWaukesha => {
                load_parcels_county(&ctx.paths.db_path, &ctx.paths.waukesha_config)
            }
            StepId::FetchParcelsMilwaukee => {
                load_parcels_county(&ctx.paths.db_path, &ctx.paths.milwaukee_config)
            }
            StepId::RebuildCurrentParcels => rebuild_step(&ctx.paths.db_path),
            StepId::FetchFred => load_fred_step(ctx),
            StepId::LoadMilwaukeeLiquor => load_liquor_step(&ctx.paths.db_path),
            StepId::LoadAccelaOccupancy => load_accela_step(ctx),
            StepId::LoadUsptoAssignments => load_uspto_step(ctx),
            StepId::LoadLiraApplications => load_lira_step(ctx),
            StepId::FetchAndExtractLiraPdfs => load_lira_pdfs_step(ctx),
        }
    }
}

/// Returns a skip outcome without calling the loader. `None` means run it.
pub fn preflight(step: StepId, ctx: &RefreshContext) -> Option<StepOutcome> {
    match step {
        StepId::FetchFred => {
            if ctx.fred_api_key.is_none() {
                Some(StepOutcome::needs_api_key(format!(
                    "FRED_API_KEY is not set. {FRED_API_KEY_ABSENT}"
                )))
            } else {
                None
            }
        }
        StepId::LoadAccelaOccupancy => {
            if ctx.accela_poll_start.is_none() {
                Some(StepOutcome::skipped(ACCELA_SINCE_NEEDED.to_string()))
            } else {
                None
            }
        }
        StepId::LoadUsptoAssignments => {
            match list_assignment_zips(&ctx.paths.assignments_folder) {
                Ok(zips) if zips.is_empty() => {
                    let downloads = list_assignment_zips(&ctx.paths.assignments_downloads)
                        .unwrap_or_default();
                    if downloads.is_empty() {
                        Some(StepOutcome::needs_manual_file(format!(
                            "place adYYYYMMDD.zip in {} (or Downloads); get files from {USPTO_DOWNLOAD_PAGE}",
                            ctx.paths.assignments_folder.display()
                        )))
                    } else {
                        None
                    }
                }
                Ok(_) => None,
                Err(err) => Some(StepOutcome::failed(err.to_string())),
            }
        }
        StepId::LoadLiraApplications => match list_lira_csvs(&ctx.paths.lira_csv_folder) {
            Ok(files) if files.is_empty() => Some(StepOutcome::needs_manual_file(format!(
                "place the LIRA CSV export in {}",
                ctx.paths.lira_csv_folder.display()
            ))),
            Ok(_) => None,
            Err(err) => Some(StepOutcome::failed(err.to_string())),
        },
        StepId::FetchAndExtractLiraPdfs => {
            let urls = ctx.paths.urls_txt();
            if !urls.is_file() {
                Some(StepOutcome::needs_manual_file(format!(
                    "place ApplAttachServlet URLs in {}",
                    urls.display()
                )))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn run_refresh<E, W>(
    out: &mut W,
    ctx: &RefreshContext,
    executor: &mut E,
) -> Result<RefreshReport, io::Error>
where
    E: StepExecutor,
    W: Write,
{
    let mut steps = Vec::new();
    for step in SEQUENCE {
        writeln!(out, "REFRESH start source={}", step.source_name())?;
        out.flush()?;
        let started = Instant::now();
        let outcome = if let Some(skip) = preflight(step, ctx) {
            skip
        } else {
            executor.execute(step, ctx)
        };
        let elapsed = started.elapsed();
        let count_or_reason = outcome.count_or_reason();
        writeln!(
            out,
            "REFRESH {} source={} count_or_reason={} elapsed_s={:.1}",
            outcome.status.as_str(),
            step.source_name(),
            count_or_reason,
            elapsed.as_secs_f64()
        )?;
        out.flush()?;
        steps.push(StepReport {
            step,
            status: outcome.status,
            count_or_reason,
            elapsed,
        });
    }
    writeln!(out)?;
    writeln!(out, "REFRESH summary")?;
    writeln!(
        out,
        "{:<28} {:<20} {:<72} {:>10}",
        "source", "status", "count_or_reason", "elapsed"
    )?;
    for row in &steps {
        writeln!(
            out,
            "{:<28} {:<20} {:<72} {:>9.1}s",
            row.step.source_name(),
            row.status.as_str(),
            truncate(&row.count_or_reason, 72),
            row.elapsed.as_secs_f64()
        )?;
    }
    out.flush()?;
    Ok(RefreshReport { steps })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out = s.chars().take(max.saturating_sub(3)).collect::<String>();
        out.push_str("...");
        out
    }
}

fn open_migrated(db_path: &Path) -> Result<Connection, StoreError> {
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| {
                StoreError::ParcelFetch(format!("create db directory: {err}"))
            })?;
        }
    }
    let mut conn = Connection::open(db_path)?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn ready_config(dir: &Path) -> Result<radar_config::RadarConfig, StoreError> {
    match SessionState::from_dir(dir).map_err(|err| StoreError::ParcelFetch(err.to_string()))? {
        SessionState::Ready { config, warnings, .. } => {
            for warning in warnings {
                println!("config warning {}: {}", warning.rule_code, warning.message);
            }
            Ok(config)
        }
        SessionState::Invalid { findings, .. } => {
            let msg = findings
                .into_iter()
                .map(|f| format!("{}: {}", f.rule_code, f.message))
                .collect::<Vec<_>>()
                .join("; ");
            Err(StoreError::ParcelFetch(format!("config not Ready: {msg}")))
        }
        SessionState::Unconfigured => Err(StoreError::ParcelFetch(
            "config is Unconfigured".to_string(),
        )),
    }
}

fn load_parcels_county(db_path: &Path, config_dir: &Path) -> StepOutcome {
    let config = match ready_config(config_dir) {
        Ok(config) => config,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    let mut conn = match open_migrated(db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    let source = LiveParcelSource::new();
    let fetched_at = rfc3339_now();
    match load_parcels(&mut conn, &source, &fetched_at, &config.entity) {
        Ok(result) => StepOutcome::succeeded(result.records_written),
        Err(err) => StepOutcome::failed(err.to_string()),
    }
}

fn rebuild_step(db_path: &Path) -> StepOutcome {
    let conn = match open_migrated(db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    match rebuild_current_parcels(&conn, None) {
        Ok(n) => StepOutcome::succeeded(n),
        Err(err) => StepOutcome::failed(err.to_string()),
    }
}

fn load_fred_step(ctx: &RefreshContext) -> StepOutcome {
    let config = match ready_config(&ctx.paths.waukesha_config) {
        Ok(config) => config,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    let client = match LiveFredClient::from_env() {
        Ok(client) => client,
        Err(err) => return StepOutcome::needs_api_key(err.to_string()),
    };
    let mut conn = match open_migrated(&ctx.paths.db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    match load_fred_series(&mut conn, &config.series, &client) {
        Ok(result) => {
            let written: i64 = conn
                .query_row(
                    "SELECT COALESCE(records_written, 0) FROM load_runs WHERE id = ?1",
                    [result.load_run_id],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            StepOutcome::succeeded(written)
        }
        Err(err) => StepOutcome::failed(err.to_string()),
    }
}

fn load_liquor_step(db_path: &Path) -> StepOutcome {
    let mut conn = match open_migrated(db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    let (fetch, loaded) = match fetch_and_load_liquor(&mut conn) {
        Ok(result) => result,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    println!("fetch status={}", fetch.status);
    println!(
        "today snapshot_date={} load_run_id={} row_count={} already_loaded={}",
        loaded.snapshot_date, loaded.load_run_id, loaded.row_count, loaded.already_loaded
    );
    match detect_liquor_latest(&conn) {
        Ok(diff) => {
            print_liquor_diff(&diff);
            match diff.comparability {
                LiquorComparability::Comparable => {
                    if let Err(err) = print_liquor_detections(&conn, &diff) {
                        eprintln!("print detections: {err}");
                    }
                }
                _ => {
                    println!("liquor detect not comparable; snapshot still stored insert-only");
                }
            }
        }
        Err(err) => eprintln!("detect: {err}"),
    }
    StepOutcome::succeeded(loaded.row_count)
}

fn load_accela_step(ctx: &RefreshContext) -> StepOutcome {
    let start = match &ctx.accela_poll_start {
        Some(start) => start.clone(),
        None => return StepOutcome::skipped(ACCELA_SINCE_NEEDED.to_string()),
    };
    let mut conn = match open_migrated(&ctx.paths.db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    let fetched_at = rfc3339_now();
    println!("poll Start Date={start} End Date=(empty, through today)");
    match fetch_and_load_accela_occupancy(&mut conn, &start, None, &fetched_at) {
        Ok(result) => {
            print_accela_load(&result);
            match detect_occupancy_new_business(&conn) {
                Ok(hits) => println!(
                    "occupancy_new_business hits={} (original table, insert-only)",
                    hits.len()
                ),
                Err(err) => eprintln!("{err}"),
            }
            if let Some(load_run_id) = result.load_run_id {
                match detect_occupancy_new_business_for_load(&conn, load_run_id) {
                    Ok(hits) => println!(
                        "occupancy_new_business snapshot_hits={} load_run_id={}",
                        hits.len(),
                        load_run_id
                    ),
                    Err(err) => eprintln!("{err}"),
                }
            }
            StepOutcome::succeeded(result.stored_rows)
        }
        Err(err) => StepOutcome::failed(err.to_string()),
    }
}

fn load_uspto_step(ctx: &RefreshContext) -> StepOutcome {
    match prepare_assignments_folder(
        &ctx.paths.assignments_folder,
        &ctx.paths.assignments_downloads,
    ) {
        Ok(moved) => {
            if !moved.is_empty() {
                println!(
                    "moved {} zip(s) into {}",
                    moved.len(),
                    ctx.paths.assignments_folder.display()
                );
            }
        }
        Err(err) => return StepOutcome::failed(err.to_string()),
    }
    let zips = match list_assignment_zips(&ctx.paths.assignments_folder) {
        Ok(zips) => zips,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    if zips.is_empty() {
        return StepOutcome::needs_manual_file(format!(
            "place adYYYYMMDD.zip in {} ; get files from {USPTO_DOWNLOAD_PAGE}",
            ctx.paths.assignments_folder.display()
        ));
    }
    let config = match ready_config(&ctx.paths.waukesha_config) {
        Ok(config) => config,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    let mut conn = match open_migrated(&ctx.paths.db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    match load_assignments_from_folder(&mut conn, &ctx.paths.assignments_folder, &config.assignments)
    {
        Ok(result) => {
            let stored: i64 = result.files.iter().map(|f| f.stored).sum();
            StepOutcome::succeeded(stored)
        }
        Err(err) => StepOutcome::failed(err.to_string()),
    }
}

fn load_lira_step(ctx: &RefreshContext) -> StepOutcome {
    let files = match list_lira_csvs(&ctx.paths.lira_csv_folder) {
        Ok(files) => files,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    if files.is_empty() {
        return StepOutcome::needs_manual_file(format!(
            "place the LIRA CSV export in {}",
            ctx.paths.lira_csv_folder.display()
        ));
    }
    let mut conn = match open_migrated(&ctx.paths.db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    match load_lira_from_folder(&mut conn, &ctx.paths.lira_csv_folder) {
        Ok(result) => StepOutcome::succeeded(result.stored_rows),
        Err(err) => StepOutcome::failed(err.to_string()),
    }
}

fn load_lira_pdfs_step(ctx: &RefreshContext) -> StepOutcome {
    let urls = ctx.paths.urls_txt();
    if !urls.is_file() {
        return StepOutcome::needs_manual_file(format!(
            "place ApplAttachServlet URLs in {}",
            urls.display()
        ));
    }
    match fetch_pdfs_from_url_file(&ctx.paths.lira_pdf_folder, &LivePdfHttp) {
        Ok(result) => {
            print_pdf_fetch(&result);
            if result
                .rows
                .iter()
                .any(|row| matches!(row.status, crate::lira_pdf::PdfFetchStatus::Failed))
            {
                return StepOutcome::failed("one or more PDF URLs failed to download".to_string());
            }
        }
        Err(err) => return StepOutcome::failed(err.to_string()),
    }
    let pdfs = match list_lira_pdfs(&ctx.paths.lira_pdf_folder) {
        Ok(pdfs) => pdfs,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    if pdfs.is_empty() {
        return StepOutcome::needs_manual_file(format!(
            "no application PDFs in {}",
            ctx.paths.lira_pdf_folder.display()
        ));
    }
    if ctx.anthropic_api_key.is_none() {
        return StepOutcome::needs_api_key(format!(
            "ANTHROPIC_API_KEY is not set. {ANTHROPIC_API_KEY_ABSENT}"
        ));
    }
    let client = match AnthropicClient::from_env() {
        Ok(client) => client,
        Err(err) => return StepOutcome::needs_api_key(err.to_string()),
    };
    let model = anthropic_model_from_env();
    let mut conn = match open_migrated(&ctx.paths.db_path) {
        Ok(conn) => conn,
        Err(err) => return StepOutcome::failed(err.to_string()),
    };
    match extract_folder(
        &mut conn,
        &client,
        &model,
        &ctx.paths.lira_pdf_folder,
        &ctx.paths.rasterize_script,
    ) {
        Ok(records) => StepOutcome::succeeded(records.len() as i64),
        Err(err) => StepOutcome::failed(err.to_string()),
    }
}

fn rfc3339_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let nanos = now.subsec_nanos();
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let (y, m, d) = civil_ymd(days);
    let hour = sod / 3600;
    let min = (sod % 3600) / 60;
    let sec = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}.{nanos:09}+00:00")
}

fn civil_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 }.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingExecutor {
        executed: Vec<StepId>,
    }

    impl RecordingExecutor {
        fn new() -> Self {
            Self {
                executed: Vec::new(),
            }
        }
    }

    impl StepExecutor for RecordingExecutor {
        fn execute(&mut self, step: StepId, _ctx: &RefreshContext) -> StepOutcome {
            self.executed.push(step);
            StepOutcome::succeeded(1)
        }
    }

    fn fixture_ctx(root: &Path, accela: Option<&str>, fred: bool) -> RefreshContext {
        let paths = RefreshPaths {
            db_path: root.join("radar.db"),
            waukesha_config: root.join("waukesha"),
            milwaukee_config: root.join("milwaukee"),
            assignments_folder: root.join("uspto"),
            assignments_downloads: root.join("downloads"),
            lira_csv_folder: root.join("lira_csv"),
            lira_pdf_folder: root.join("lira_pdfs"),
            rasterize_script: root.join("rasterize_pdf.py"),
        };
        fs::create_dir_all(&paths.assignments_folder).unwrap();
        fs::create_dir_all(&paths.assignments_downloads).unwrap();
        fs::create_dir_all(&paths.lira_csv_folder).unwrap();
        fs::create_dir_all(&paths.lira_pdf_folder).unwrap();
        RefreshContext {
            paths,
            fred_api_key: if fred {
                Some("test-key".to_string())
            } else {
                None
            },
            anthropic_api_key: None,
            accela_poll_start: accela.map(|s| s.to_string()),
        }
    }

    fn unique_root() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "radar-refresh-all-{}-{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn missing_lira_csv_is_skipped_named_in_summary_and_does_not_block_later_steps() {
        let root = unique_root();
        let ctx = fixture_ctx(&root, Some("09/18/2026"), true);
        let mut executor = RecordingExecutor::new();
        let mut out = Vec::new();
        let report = run_refresh(&mut out, &ctx, &mut executor).expect("run");
        let text = String::from_utf8(out).unwrap();

        let lira = report
            .steps
            .iter()
            .find(|s| s.step == StepId::LoadLiraApplications)
            .expect("lira in summary");
        assert_eq!(lira.status, StepStatus::NeedsManualFile);
        assert!(
            lira.count_or_reason.contains(&ctx.paths.lira_csv_folder.display().to_string()),
            "skip reason must name the CSV folder, got {}",
            lira.count_or_reason
        );
        assert!(text.contains("lira_applications"));
        assert!(text.contains("needs manual file"));

        let pdf = report
            .steps
            .iter()
            .find(|s| s.step == StepId::FetchAndExtractLiraPdfs)
            .expect("pdf step still in summary");
        assert_eq!(pdf.status, StepStatus::NeedsManualFile);
        assert!(pdf
            .count_or_reason
            .contains(&ctx.paths.urls_txt().display().to_string()));

        assert!(
            executor
                .executed
                .contains(&StepId::LoadAccelaOccupancy),
            "Accela must still run after earlier skips; executed={:?}",
            executor.executed
        );
        assert!(
            !executor
                .executed
                .contains(&StepId::LoadLiraApplications),
            "missing CSV must not call the LIRA loader"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sequence_rebuilds_after_both_parcel_fetches_and_accela_after_rebuild() {
        let root = unique_root();
        let ctx = fixture_ctx(&root, Some("09/18/2026"), true);
        let mut executor = RecordingExecutor::new();
        let mut out = Vec::new();
        let report = run_refresh(&mut out, &ctx, &mut executor).expect("run");

        let executed = &executor.executed;
        let waukesha = executed
            .iter()
            .position(|s| *s == StepId::FetchParcelsWaukesha)
            .expect("waukesha fetch");
        let milwaukee = executed
            .iter()
            .position(|s| *s == StepId::FetchParcelsMilwaukee)
            .expect("milwaukee fetch");
        let rebuild = executed
            .iter()
            .position(|s| *s == StepId::RebuildCurrentParcels)
            .expect("rebuild");
        let accela = executed
            .iter()
            .position(|s| *s == StepId::LoadAccelaOccupancy)
            .expect("accela");
        assert!(rebuild > waukesha, "rebuild after Waukesha parcels");
        assert!(rebuild > milwaukee, "rebuild after Milwaukee parcels");
        assert!(
            accela > rebuild,
            "Accela must not run before rebuild_current_parcels"
        );

        let names: Vec<_> = report.steps.iter().map(|s| s.step).collect();
        assert_eq!(names, SEQUENCE.to_vec());
        let _ = fs::remove_dir_all(&root);
    }
}
