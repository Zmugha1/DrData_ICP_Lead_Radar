use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::thread;
use std::time::{Duration, SystemTime};

use radar_config::{SeriesEntry, SeriesFile};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

use crate::error::StoreError;

pub const FRED_SOURCE_ID: &str = "src-fred";
pub const FRED_API_KEY_ABSENT: &str = "FRED_API_KEY is absent. The key is read only from that environment variable. No network call will be made. The key is not prompted for and is not written to any file or config directory.";
const FRED_MISSING: &str = ".";
const OBS_PAGE_LIMIT: usize = 10_000;
const LIVE_GAP: Duration = Duration::from_millis(600);

pub fn fred_api_key_from_env() -> Option<String> {
    match std::env::var("FRED_API_KEY") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

#[derive(Clone)]
pub struct FredHttpResponse {
    pub status: u16,
    pub body: String,
    pub url_redacted: String,
}

pub trait FredClient {
    fn fetch_series_meta(&self, series_id: &str) -> Result<FredHttpResponse, StoreError>;
    fn fetch_observations(
        &self,
        series_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<FredHttpResponse, StoreError>;
}

pub struct LiveFredClient {
    api_key: String,
}

impl LiveFredClient {
    pub fn from_env() -> Result<Self, StoreError> {
        match fred_api_key_from_env() {
            Some(api_key) => Ok(Self { api_key }),
            None => Err(StoreError::FredFetch(FRED_API_KEY_ABSENT.to_string())),
        }
    }

    fn redact(&self, text: &str) -> String {
        text.replace(&self.api_key, "REDACTED")
    }

    fn get_once(&self, url: &str) -> Result<FredHttpResponse, StoreError> {
        let url_redacted = self.redact(url);
        match ureq::get(url).timeout(Duration::from_secs(60)).call() {
            Ok(response) => {
                let status = response.status();
                let body = response
                    .into_string()
                    .map_err(|err| StoreError::FredFetch(self.redact(&err.to_string())))?;
                Ok(FredHttpResponse {
                    status,
                    body,
                    url_redacted,
                })
            }
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                Ok(FredHttpResponse {
                    status,
                    body,
                    url_redacted,
                })
            }
            Err(err) => Err(StoreError::FredFetch(self.redact(&err.to_string()))),
        }
    }

    fn get_with_retry(
        &self,
        endpoint: &str,
        series_id: &str,
        url: &str,
    ) -> Result<FredHttpResponse, StoreError> {
        let url_redacted = self.redact(url);
        let mut last_status = 0u16;
        for attempt in 1..=3 {
            println!("FRED request endpoint={endpoint} series_id={series_id} attempt={attempt}");
            match self.get_once(url) {
                Ok(response) => {
                    println!(
                        "FRED status endpoint={endpoint} series_id={series_id} status={}",
                        response.status
                    );
                    if response.status == 200 {
                        thread::sleep(LIVE_GAP);
                        return Ok(response);
                    }
                    last_status = response.status;
                    println!(
                        "FRED fail endpoint={endpoint} series_id={series_id} status={} url={url_redacted}",
                        response.status
                    );
                }
                Err(err) => {
                    println!(
                        "FRED fail endpoint={endpoint} series_id={series_id} status=error url={url_redacted} detail={err}"
                    );
                }
            }
            thread::sleep(LIVE_GAP);
        }
        Err(StoreError::FredFetch(format!(
            "series {series_id} endpoint={endpoint} failed after 3 attempts last_status={last_status} url={url_redacted}"
        )))
    }
}

impl FredClient for LiveFredClient {
    fn fetch_series_meta(&self, series_id: &str) -> Result<FredHttpResponse, StoreError> {
        let url = format!(
            "https://api.stlouisfed.org/fred/series?series_id={series_id}&api_key={}&file_type=json",
            self.api_key
        );
        self.get_with_retry("series", series_id, &url)
    }

    fn fetch_observations(
        &self,
        series_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<FredHttpResponse, StoreError> {
        let url = format!(
            "https://api.stlouisfed.org/fred/series/observations?series_id={series_id}&api_key={}&file_type=json&offset={offset}&limit={limit}",
            self.api_key
        );
        self.get_with_retry("observations", series_id, &url)
    }
}

#[derive(Debug, Clone)]
struct CatalogRow {
    series_id: String,
    title: String,
    units: String,
    frequency: String,
    seasonal_adjustment: String,
    geography: String,
    notes: Option<String>,
}

#[derive(Debug, Clone)]
struct ObservationRow {
    series_id: String,
    observation_date: String,
    value: Option<f64>,
    realtime_start: String,
    realtime_end: String,
}

#[derive(Debug, Clone)]
pub struct FredLoadResult {
    pub load_run_id: i64,
    pub assertion_report: String,
    pub profile_report: String,
    pub vintage_report: String,
    pub summary_report: String,
}

#[derive(Deserialize)]
struct FredSeriesEnvelope {
    seriess: Vec<FredSeriesMeta>,
}

#[derive(Deserialize)]
struct FredSeriesMeta {
    id: String,
    title: String,
    units: String,
    frequency: String,
    seasonal_adjustment: String,
    notes: Option<String>,
}

#[derive(Deserialize)]
struct FredObsEnvelope {
    count: Option<i64>,
    observations: Vec<FredObs>,
}

#[derive(Deserialize)]
struct FredObs {
    realtime_start: String,
    realtime_end: String,
    date: String,
    value: String,
}

pub fn parse_observation_value(raw: &str) -> Result<Option<f64>, String> {
    if raw == FRED_MISSING {
        return Ok(None);
    }
    raw.parse::<f64>()
        .map(Some)
        .map_err(|_| format!("value {raw:?} does not parse as a number and is not the missing-value marker"))
}

pub fn load_fred_series(
    conn: &mut Connection,
    series_file: &SeriesFile,
    client: &dyn FredClient,
) -> Result<FredLoadResult, StoreError> {
    let included: Vec<&SeriesEntry> = series_file.series.iter().filter(|entry| entry.included).collect();
    if included.is_empty() {
        return Err(StoreError::FredFetch(
            "no series are included for this vertical".to_string(),
        ));
    }

    let fetched_at = now_rfc3339();
    let mut catalogs = Vec::new();
    let mut observations = Vec::new();

    for entry in &included {
        let meta_http = client.fetch_series_meta(&entry.series_id)?;
        if meta_http.url_redacted.is_empty() {
            println!(
                "FRED request endpoint=series series_id={} status={}",
                entry.series_id, meta_http.status
            );
        }
        if meta_http.status != 200 {
            println!(
                "FRED fail endpoint=series series_id={} status={} url={}",
                entry.series_id, meta_http.status, meta_http.url_redacted
            );
            return Err(StoreError::FredFetch(format!(
                "series {} metadata HTTP {} url={}",
                entry.series_id, meta_http.status, meta_http.url_redacted
            )));
        }
        let envelope: FredSeriesEnvelope = serde_json::from_str(&meta_http.body)
            .map_err(|err| StoreError::FredFetch(format!("series {} metadata JSON: {err}", entry.series_id)))?;
        let meta = envelope.seriess.into_iter().next().ok_or_else(|| {
            StoreError::FredFetch(format!("series {} metadata returned no series object", entry.series_id))
        })?;
        let mut notes = entry.notes.clone();
        if notes.is_none() {
            notes = meta.notes.clone();
        }
        catalogs.push(CatalogRow {
            series_id: meta.id,
            title: meta.title,
            units: meta.units,
            frequency: meta.frequency,
            seasonal_adjustment: meta.seasonal_adjustment,
            geography: entry.geography.clone(),
            notes,
        });

        let mut offset = 0usize;
        loop {
            let obs_http = client.fetch_observations(&entry.series_id, offset, OBS_PAGE_LIMIT)?;
            if obs_http.url_redacted.is_empty() {
                println!(
                    "FRED request endpoint=observations series_id={} offset={offset} status={}",
                    entry.series_id, obs_http.status
                );
            }
            if obs_http.status != 200 {
                println!(
                    "FRED fail endpoint=observations series_id={} status={} url={}",
                    entry.series_id, obs_http.status, obs_http.url_redacted
                );
                return Err(StoreError::FredFetch(format!(
                    "series {} observations HTTP {} url={}",
                    entry.series_id, obs_http.status, obs_http.url_redacted
                )));
            }
            let page: FredObsEnvelope = serde_json::from_str(&obs_http.body).map_err(|err| {
                StoreError::FredFetch(format!("series {} observations JSON: {err}", entry.series_id))
            })?;
            let page_len = page.observations.len();
            for obs in page.observations {
                let value = parse_observation_value(&obs.value).map_err(|message| {
                    let detail = format!(
                        "S5 {} date={} expected=full-stop-or-number actual={:?} FAIL {message}",
                        entry.series_id, obs.date, obs.value
                    );
                    println!("{detail}");
                    StoreError::FredAssertion {
                        code: "S5".to_string(),
                        message: detail,
                    }
                })?;
                observations.push(ObservationRow {
                    series_id: entry.series_id.clone(),
                    observation_date: obs.date,
                    value,
                    realtime_start: obs.realtime_start,
                    realtime_end: obs.realtime_end,
                });
            }
            let total = page.count.unwrap_or(offset as i64 + page_len as i64) as usize;
            offset += page_len;
            if page_len == 0 || offset >= total {
                break;
            }
        }
    }

    let assertion_report = assert_before_commit(&included, &observations)?;
    println!("{assertion_report}");

    let started_at = fetched_at.clone();
    let finished_at = now_rfc3339();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO load_runs (
             source_id, started_at, finished_at, status,
             records_requested, records_written, records_skipped
         ) VALUES (?1, ?2, ?3, 'success', ?4, ?5, 0)",
        params![
            FRED_SOURCE_ID,
            started_at,
            finished_at,
            observations.len() as i64,
            observations.len() as i64
        ],
    )?;
    let load_run_id = tx.last_insert_rowid();

    for catalog in &catalogs {
        tx.execute(
            "INSERT INTO series_catalog (
                 series_id, title, units, frequency, seasonal_adjustment,
                 geography, publication_lag_days, lag_measured, notes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, ?7)",
            params![
                catalog.series_id,
                catalog.title,
                catalog.units,
                catalog.frequency,
                catalog.seasonal_adjustment,
                catalog.geography,
                catalog.notes
            ],
        )?;
    }
    for row in &observations {
        tx.execute(
            "INSERT INTO series_observations (
                 series_id, observation_date, value, realtime_start, realtime_end,
                 load_run_id, fetched_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.series_id,
                row.observation_date,
                row.value,
                row.realtime_start,
                row.realtime_end,
                load_run_id,
                fetched_at
            ],
        )?;
    }
    tx.commit()?;

    let profile_report = format_series_profile(conn)?;
    let vintage_report = format_vintage_report(conn)?;
    let summary_report = format_honest_summary(conn)?;
    println!("{profile_report}");
    println!("{vintage_report}");
    println!("{summary_report}");

    Ok(FredLoadResult {
        load_run_id,
        assertion_report,
        profile_report,
        vintage_report,
        summary_report,
    })
}

fn assert_before_commit(
    included: &[&SeriesEntry],
    observations: &[ObservationRow],
) -> Result<String, StoreError> {
    let mut lines = Vec::new();
    lines.push("TASK 2 ASSERTIONS".to_string());

    let mut by_series: BTreeMap<String, Vec<&ObservationRow>> = BTreeMap::new();
    for row in observations {
        by_series
            .entry(row.series_id.clone())
            .or_default()
            .push(row);
    }

    for entry in included {
        let rows = by_series.get(&entry.series_id);
        let count = rows.map(|rows| rows.len()).unwrap_or(0);
        if count == 0 {
            let detail = format!(
                "S1 {} expected>=1 actual=0 FAIL included series returned no observations",
                entry.series_id
            );
            println!("{detail}");
            return Err(StoreError::FredAssertion {
                code: "S1".to_string(),
                message: detail,
            });
        }
        let dates: Vec<&str> = rows
            .expect("count > 0")
            .iter()
            .map(|row| row.observation_date.as_str())
            .collect();
        let min_date = dates.iter().min().copied().unwrap_or("");
        let max_date = dates.iter().max().copied().unwrap_or("");
        lines.push(format!(
            "S1 {} expected>=1 actual={count} min_date={min_date} max_date={max_date} PASS",
            entry.series_id
        ));
    }

    let mut seen = HashSet::new();
    let mut duplicates = 0i64;
    let mut first_dup = String::new();
    for row in observations {
        let key = (
            row.series_id.clone(),
            row.observation_date.clone(),
            row.realtime_start.clone(),
        );
        if !seen.insert(key) {
            duplicates += 1;
            if first_dup.is_empty() {
                first_dup = format!(
                    "series_id={} observation_date={} realtime_start={}",
                    row.series_id, row.observation_date, row.realtime_start
                );
            }
        }
    }
    if duplicates > 0 {
        let detail = format!(
            "S3 expected=0 duplicate keys actual={duplicates} FAIL first={first_dup}"
        );
        println!("{detail}");
        return Err(StoreError::FredAssertion {
            code: "S3".to_string(),
            message: detail,
        });
    }
    lines.push(format!(
        "S3 expected=0 duplicate on (series_id, observation_date, realtime_start) actual=0 rows={} PASS",
        observations.len()
    ));

    for entry in included {
        let nulls = by_series
            .get(&entry.series_id)
            .map(|rows| rows.iter().filter(|row| row.value.is_none()).count())
            .unwrap_or(0);
        lines.push(format!(
            "S4 {} expected=printed_not_failed actual_null_count={nulls}",
            entry.series_id
        ));
    }

    lines.push(format!(
        "S5 expected=full-stop-or-number actual=all_parsed rows={} PASS",
        observations.len()
    ));
    Ok(lines.join("\n"))
}

pub fn format_series_profile(conn: &Connection) -> Result<String, StoreError> {
    let mut lines = Vec::new();
    lines.push("TASK 5 PROFILE".to_string());
    let mut stmt = conn.prepare(
        "SELECT series_id, title, units, frequency, seasonal_adjustment, geography
         FROM series_catalog
         ORDER BY series_id",
    )?;
    let catalogs: Vec<(String, String, String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    if catalogs.is_empty() {
        lines.push("no series_catalog rows".to_string());
        return Ok(lines.join("\n"));
    }

    for (series_id, title, units, frequency, seasonal_adjustment, geography) in catalogs {
        let stats: (i64, Option<String>, Option<String>, i64) = conn.query_row(
            "SELECT COUNT(*), MIN(observation_date), MAX(observation_date),
                    SUM(CASE WHEN value IS NULL THEN 1 ELSE 0 END)
             FROM series_observations
             WHERE series_id = ?1",
            [&series_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let dates = observation_dates(conn, &series_id)?;
        let observed = observed_frequency(&dates);
        let declared_kind = declared_cadence(&frequency);
        let observed_kind = observed_cadence_kind(&dates);
        let frequency_match = declared_kind == observed_kind;
        lines.push(format!("series_id={series_id}"));
        lines.push(format!("  title={title}"));
        lines.push(format!("  units={units}"));
        lines.push(format!(
            "  catalog_frequency={frequency} seasonal_adjustment={seasonal_adjustment} geography={geography}"
        ));
        lines.push(format!(
            "  observation_count={} min_date={} max_date={}",
            stats.0,
            stats.1.as_deref().unwrap_or("none"),
            stats.2.as_deref().unwrap_or("none"),
        ));
        lines.push(format!("  frequency_as_observed={observed}"));
        lines.push(format!(
            "  frequency_match={} declared_kind={declared_kind} observed_kind={observed_kind}",
            if frequency_match { "yes" } else { "no" }
        ));
        let null_dates = null_observation_dates(conn, &series_id)?;
        lines.push(format!("  null_count={}", stats.3));
        if null_dates.is_empty() {
            lines.push("  null_dates=none".to_string());
        } else {
            lines.push(format!("  null_dates={}", null_dates.join(", ")));
        }
        lines.push("  ten_most_recent:".to_string());
        for (date, value) in n_most_recent(conn, &series_id, 10)? {
            let shown = match value {
                Some(number) => number.to_string(),
                None => "null".to_string(),
            };
            lines.push(format!("    {date} {shown}"));
        }
        let lower = frequency.to_lowercase();
        if lower.contains("month") {
            let gaps = monthly_gaps(&dates);
            if gaps.is_empty() {
                lines.push("  cadence_gaps=none".to_string());
            } else {
                lines.push(format!("  cadence_gaps(missing_months)={}", gaps.join(", ")));
            }
        } else if lower.contains("week") {
            let gaps = weekly_gaps(&dates);
            if gaps.is_empty() {
                lines.push("  cadence_gaps=none".to_string());
            } else {
                lines.push(format!("  cadence_gaps(missing_weeks)={}", gaps.join(", ")));
            }
        } else {
            lines.push("  cadence_gaps=not_applicable (neither weekly nor monthly)".to_string());
        }
        lines.push(String::new());
    }

    lines.push("BPPRIV055133 COUNTY DETAIL".to_string());
    let county_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM series_observations WHERE series_id = 'BPPRIV055133'",
        [],
        |row| row.get(0),
    )?;
    if county_count == 0 {
        lines.push("BPPRIV055133 is absent from series_observations (not loaded or excluded).".to_string());
    } else {
        let (min_date, max_date): (String, String) = conn.query_row(
            "SELECT MIN(observation_date), MAX(observation_date)
             FROM series_observations WHERE series_id = 'BPPRIV055133'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        lines.push(format!(
            "full history count={county_count} range={min_date} to {max_date}"
        ));
        let mut hist = conn.prepare(
            "SELECT observation_date, value
             FROM series_observations
             WHERE series_id = 'BPPRIV055133'
             ORDER BY observation_date",
        )?;
        let history: Vec<(String, Option<f64>)> = hist
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        lines.push("full history:".to_string());
        for (date, value) in &history {
            let shown = match value {
                Some(number) => number.to_string(),
                None => "null".to_string(),
            };
            lines.push(format!("  {date} {shown}"));
        }
        lines.push("last_twenty:".to_string());
        for (date, value) in n_most_recent(conn, "BPPRIV055133", 20)? {
            let shown = match value {
                Some(number) => number.to_string(),
                None => "null".to_string(),
            };
            lines.push(format!("  {date} {shown}"));
        }
        let today = today_ymd();
        let lag_days = days_between(&max_date, &today).unwrap_or(0);
        lines.push(format!("most_recent_observation_date={max_date}"));
        lines.push(format!(
            "publication lag measured as today ({today}) minus newest observation_date ({max_date}) = {lag_days} days"
        ));
        lines.push(format!(
            "The newest Waukesha County figure (BPPRIV055133) is {lag_days} days stale relative to today."
        ));
        if lag_days > 365 {
            lines.push(
                "The newest figure is more than a year old. That belongs beside the number, not discovered by a client."
                    .to_string(),
            );
        }
        lines.push(
            "This lag is today minus the newest observation_date on the current observations endpoint, not an ALFRED first-vintage delay."
                .to_string(),
        );
        if (480..=508).contains(&lag_days) {
            lines.push(
                "That confirms the earlier ALFRED first-vintage profile of 480 to 508 days: the newest county figure is in the same stale band."
                    .to_string(),
            );
        } else {
            lines.push(format!(
                "That corrects the earlier ALFRED first-vintage profile of 480 to 508 days; this fetch measures {lag_days} days from observation date to today."
            ));
        }
    }

    Ok(lines.join("\n"))
}

pub fn format_vintage_report(conn: &Connection) -> Result<String, StoreError> {
    let mut lines = Vec::new();
    lines.push("TASK 5 VINTAGE (ASKED, NOT IMPLEMENTED)".to_string());
    lines.push(
        "ALFRED exposes vintages. This fetch used the plain observations endpoint, which returns the current vintage."
            .to_string(),
    );
    let mut series_stmt = conn.prepare(
        "SELECT DISTINCT series_id FROM series_observations ORDER BY series_id",
    )?;
    let series_ids: Vec<String> = series_stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;

    if series_ids.is_empty() {
        lines.push("no observations; vintage fields were not observed.".to_string());
        return Ok(lines.join("\n"));
    }

    let mut any_varies = false;
    for series_id in &series_ids {
        let mut pair_stmt = conn.prepare(
            "SELECT realtime_start, realtime_end, COUNT(*)
             FROM series_observations
             WHERE series_id = ?1
             GROUP BY realtime_start, realtime_end
             ORDER BY COUNT(*) DESC, realtime_start, realtime_end",
        )?;
        let pairs: Vec<(String, String, i64)> = pair_stmt
            .query_map([series_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let varies = pairs.len() > 1;
        any_varies |= varies;
        lines.push(format!(
            "{series_id} distinct_realtime_pairs={} varies_within_series={varies}",
            pairs.len()
        ));
        for (start, end, count) in &pairs {
            lines.push(format!(
                "  realtime_start={start} realtime_end={end} count={count}"
            ));
        }
        if varies {
            let example: Option<(String, i64)> = conn
                .query_row(
                    "SELECT observation_date, COUNT(*)
                     FROM series_observations
                     WHERE series_id = ?1
                     GROUP BY observation_date
                     HAVING COUNT(DISTINCT realtime_start || '|' || realtime_end) > 1
                     ORDER BY observation_date
                     LIMIT 1",
                    [series_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            match example {
                Some((date, count)) => lines.push(format!(
                    "  example observation_date={date} carries {count} rows with more than one realtime range"
                )),
                None => lines.push(
                    "  pairs vary across the series but no single observation_date carries more than one realtime range"
                        .to_string(),
                ),
            }
        }
    }
    if any_varies {
        lines.push(
            "realtime_start or realtime_end varied within at least one series on this fetch. Revisions are visible in this payload as multiple realtime ranges. Vintage handling is not implemented."
                .to_string(),
        );
    } else {
        lines.push(
            "Every observation in each series carries the same realtime_start/realtime_end pair. The observations endpoint returned one vintage; ALFRED would be needed to see revisions. Vintage handling is not implemented."
                .to_string(),
        );
    }
    Ok(lines.join("\n"))
}

pub fn format_honest_summary(conn: &Connection) -> Result<String, StoreError> {
    let mut lines = Vec::new();
    lines.push("TASK 6 HONEST SUMMARY".to_string());
    let mut stmt = conn.prepare(
        "SELECT c.series_id, c.geography, c.frequency,
                COUNT(o.observation_date),
                MIN(o.observation_date), MAX(o.observation_date)
         FROM series_catalog c
         LEFT JOIN series_observations o ON o.series_id = c.series_id
         GROUP BY c.series_id, c.geography, c.frequency
         ORDER BY c.series_id",
    )?;
    let rows: Vec<(String, String, String, i64, Option<String>, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let mut national = Vec::new();
    let mut state = Vec::new();
    let mut county = Vec::new();
    let mut gap_series = Vec::new();
    for (series_id, geography, frequency, count, min_date, max_date) in &rows {
        lines.push(format!(
            "loaded {series_id} geography={geography} observations={count} min={} max={}",
            min_date.as_deref().unwrap_or("none"),
            max_date.as_deref().unwrap_or("none")
        ));
        match geography.as_str() {
            "national" => national.push(series_id.clone()),
            "state" => state.push(series_id.clone()),
            "county" => county.push(series_id.clone()),
            _ => {}
        }
        let dates = observation_dates(conn, series_id)?;
        let lower = frequency.to_lowercase();
        let gaps = if lower.contains("month") {
            monthly_gaps(&dates)
        } else if lower.contains("week") {
            weekly_gaps(&dates)
        } else {
            Vec::new()
        };
        if !gaps.is_empty() {
            gap_series.push(format!("{series_id} ({})", gaps.len()));
        }
    }

    let (oldest, newest): (Option<String>, Option<String>) = conn.query_row(
        "SELECT MIN(observation_date), MAX(observation_date) FROM series_observations",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    lines.push(format!(
        "oldest_in_store={} newest_in_store={}",
        oldest.as_deref().unwrap_or("none"),
        newest.as_deref().unwrap_or("none")
    ));
    lines.push(format!("national: {}", national.join(", ")));
    lines.push(format!("state: {}", state.join(", ")));
    lines.push(format!("county: {}", county.join(", ")));

    let county_max: Option<String> = conn.query_row(
        "SELECT MAX(observation_date) FROM series_observations WHERE series_id = 'BPPRIV055133'",
        [],
        |row| row.get(0),
    )?;
    let today = today_ymd();
    match county_max {
        Some(max_date) => {
            let lag_days = days_between(&max_date, &today).unwrap_or(0);
            lines.push(format!(
                "county_staleness BPPRIV055133 newest={max_date} today={today} lag_days={lag_days}"
            ));
        }
        None => lines.push("county_staleness BPPRIV055133 not loaded".to_string()),
    }

    if gap_series.is_empty() {
        lines.push("gaps: none in weekly or monthly cadence checks".to_string());
    } else {
        lines.push(format!("gaps present in: {}", gap_series.join(", ")));
    }

    let varying: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (
             SELECT series_id FROM series_observations
             GROUP BY series_id
             HAVING COUNT(DISTINCT realtime_start || '|' || realtime_end) > 1
         )",
        [],
        |row| row.get(0),
    )?;
    if varying > 0 {
        lines.push(format!(
            "revisions: visible in this data; {varying} series have more than one realtime pair"
        ));
    } else {
        lines.push(
            "revisions: not visible in this data; each series has one realtime pair (current vintage only)"
                .to_string(),
        );
    }

    lines.push("action_this_week versus context_only:".to_string());
    lines.push(
        "MORTGAGE30US is the only series a builder could act on this week: it is weekly, national, and current enough to change a bid or a hold. HOUST, PERMIT, COMPUTSA, HSN1F, MSACSR, MSPNHSUS and TLRESCONS are national monthly context; they describe the country, not a Waukesha site. WIBPPRIV is Wisconsin context. BPPRIV055133 is the only county series and it is too stale to time a conversation this week; it is background, not a trigger."
            .to_string(),
    );
    Ok(lines.join("\n"))
}

pub(crate) fn observation_dates(conn: &Connection, series_id: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT observation_date FROM series_observations WHERE series_id = ?1 ORDER BY observation_date",
    )?;
    let dates = stmt
        .query_map([series_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(dates)
}

fn null_observation_dates(conn: &Connection, series_id: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT observation_date FROM series_observations
         WHERE series_id = ?1 AND value IS NULL
         ORDER BY observation_date",
    )?;
    let dates = stmt
        .query_map([series_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(dates)
}

pub(crate) fn n_most_recent(
    conn: &Connection,
    series_id: &str,
    limit: i64,
) -> Result<Vec<(String, Option<f64>)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT observation_date, value FROM series_observations
         WHERE series_id = ?1
         ORDER BY observation_date DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![series_id, limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub(crate) fn declared_cadence(frequency: &str) -> &'static str {
    let lower = frequency.to_lowercase();
    if lower.contains("week") {
        "weekly"
    } else if lower.contains("month") {
        "monthly"
    } else if lower.contains("quarter") {
        "quarterly"
    } else if lower.contains("annual") || lower.contains("year") {
        "annual"
    } else {
        "other"
    }
}

pub(crate) fn observed_cadence_kind(dates: &[String]) -> &'static str {
    if dates.len() < 2 {
        return "other";
    }
    let mut gaps = Vec::new();
    for window in dates.windows(2) {
        if let Some(days) = days_between(&window[0], &window[1]) {
            gaps.push(days);
        }
    }
    if gaps.is_empty() {
        return "other";
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    if median <= 8 {
        "weekly"
    } else if median <= 20 {
        "biweekly"
    } else if median <= 40 {
        "monthly"
    } else if median <= 100 {
        "quarterly"
    } else {
        "annual"
    }
}

fn observed_frequency(dates: &[String]) -> String {
    if dates.len() < 2 {
        return "single_observation".to_string();
    }
    let mut gaps = Vec::new();
    for window in dates.windows(2) {
        if let Some(days) = days_between(&window[0], &window[1]) {
            gaps.push(days);
        }
    }
    if gaps.is_empty() {
        return "unknown".to_string();
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    if median <= 8 {
        format!("weekly (median_gap_days={median})")
    } else if median <= 20 {
        format!("biweekly (median_gap_days={median})")
    } else if median <= 40 {
        format!("monthly (median_gap_days={median})")
    } else if median <= 100 {
        format!("quarterly (median_gap_days={median})")
    } else {
        format!("annual_or_irregular (median_gap_days={median})")
    }
}

fn monthly_gaps(dates: &[String]) -> Vec<String> {
    let months: BTreeSet<(i32, u32)> = dates
        .iter()
        .filter_map(|date| parse_ymd(date).map(|(y, m, _)| (y, m)))
        .collect();
    if months.len() < 2 {
        return Vec::new();
    }
    let start = *months.iter().next().expect("len>=2");
    let end = *months.iter().next_back().expect("len>=2");
    let mut cursor = start;
    let mut missing = Vec::new();
    while cursor <= end {
        if !months.contains(&cursor) {
            missing.push(format!("{:04}-{:02}", cursor.0, cursor.1));
        }
        cursor = add_month(cursor);
    }
    missing
}

fn weekly_gaps(dates: &[String]) -> Vec<String> {
    if dates.len() < 2 {
        return Vec::new();
    }
    let mut missing = Vec::new();
    for window in dates.windows(2) {
        let Some(days) = days_between(&window[0], &window[1]) else {
            continue;
        };
        if days <= 10 {
            continue;
        }
        let Some((y, m, d)) = parse_ymd(&window[0]) else {
            continue;
        };
        let mut cursor_days = days_from_ymd(y, m, d) + 7;
        let Some((y2, m2, d2)) = parse_ymd(&window[1]) else {
            continue;
        };
        let end_days = days_from_ymd(y2, m2, d2);
        while cursor_days < end_days {
            let (cy, cm, cd) = ymd_from_unix_days(cursor_days);
            missing.push(format!("{cy:04}-{cm:02}-{cd:02}"));
            cursor_days += 7;
        }
    }
    missing
}

fn parse_ymd(value: &str) -> Option<(i32, u32, u32)> {
    if value.len() < 10 {
        return None;
    }
    let year = value[0..4].parse().ok()?;
    let month = value[5..7].parse().ok()?;
    let day = value[8..10].parse().ok()?;
    Some((year, month, day))
}

fn add_month((year, month): (i32, u32)) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

pub(crate) fn days_between(start: &str, end: &str) -> Option<i64> {
    let (y1, m1, d1) = parse_ymd(start)?;
    let (y2, m2, d2) = parse_ymd(end)?;
    Some(days_from_ymd(y2, m2, d2) - days_from_ymd(y1, m1, d1))
}

pub(crate) fn today_ymd() -> String {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock");
    let days = now.as_secs() as i64 / 86400;
    let (y, m, d) = ymd_from_unix_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn add_days(date: &str, delta: i64) -> Option<String> {
    let (y, m, d) = parse_ymd(date)?;
    let (y, m, d) = ymd_from_unix_days(days_from_ymd(y, m, d) + delta);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

fn now_rfc3339() -> String {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock");
    let secs = now.as_secs() as i64;
    let nanos = now.subsec_nanos();
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let (y, m, d) = ymd_from_unix_days(days);
    let hour = sod / 3600;
    let min = (sod % 3600) / 60;
    let sec = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}.{nanos:09}+00:00")
}

fn ymd_from_unix_days(days: i64) -> (i32, u32, u32) {
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

fn days_from_ymd(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 }.div_euclid(400);
    let yoe = (y - era * 400) as i64;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_config::SeriesEntry;
    use rusqlite::Connection;

    use crate::migrate::migrate;

    struct MapClient {
        meta: BTreeMap<String, FredHttpResponse>,
        obs: BTreeMap<String, FredHttpResponse>,
    }

    impl FredClient for MapClient {
        fn fetch_series_meta(&self, series_id: &str) -> Result<FredHttpResponse, StoreError> {
            self.meta.get(series_id).cloned().ok_or_else(|| {
                StoreError::FredFetch(format!("no fixture meta for {series_id}"))
            })
        }

        fn fetch_observations(
            &self,
            series_id: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<FredHttpResponse, StoreError> {
            self.obs.get(series_id).cloned().ok_or_else(|| {
                StoreError::FredFetch(format!("no fixture observations for {series_id}"))
            })
        }
    }

    fn entry(id: &str, geography: &str, included: bool) -> SeriesEntry {
        SeriesEntry {
            series_id: id.to_string(),
            label: id.to_string(),
            geography: geography.to_string(),
            included,
            notes: None,
        }
    }

    fn ok(body: impl Into<String>) -> FredHttpResponse {
        FredHttpResponse {
            status: 200,
            body: body.into(),
            url_redacted: String::new(),
        }
    }

    fn meta_body(id: &str, frequency: &str) -> String {
        format!(
            r#"{{"seriess":[{{"id":"{id}","title":"{id} title","units":"Units","frequency":"{frequency}","seasonal_adjustment":"Not Seasonally Adjusted"}}]}}"#
        )
    }

    fn obs_body(rows: &[(&str, &str, &str, &str)]) -> String {
        let items: Vec<String> = rows
            .iter()
            .map(|(date, value, start, end)| {
                format!(
                    r#"{{"realtime_start":"{start}","realtime_end":"{end}","date":"{date}","value":"{value}"}}"#
                )
            })
            .collect();
        format!(
            r#"{{"count":{},"observations":[{}]}}"#,
            items.len(),
            items.join(",")
        )
    }

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn missing_marker_is_null_and_garbage_fails() {
        assert_eq!(parse_observation_value(".").unwrap(), None);
        assert_eq!(parse_observation_value("12.5").unwrap(), Some(12.5));
        assert!(parse_observation_value("NA").is_err());
        assert!(parse_observation_value("").is_err());
    }

    #[test]
    fn load_asserts_and_profiles_fixture() {
        let mut conn = open();
        let client = MapClient {
            meta: BTreeMap::from([
                ("HOUST".to_string(), ok(meta_body("HOUST", "Monthly"))),
                (
                    "MORTGAGE30US".to_string(),
                    ok(meta_body("MORTGAGE30US", "Weekly, Ending Thursday")),
                ),
            ]),
            obs: BTreeMap::from([
                (
                    "HOUST".to_string(),
                    ok(obs_body(&[
                        ("2020-01-01", "1.0", "2026-09-05", "9999-12-31"),
                        ("2020-03-01", ".", "2026-09-05", "9999-12-31"),
                        ("2020-04-01", "3.5", "2026-09-05", "9999-12-31"),
                    ])),
                ),
                (
                    "MORTGAGE30US".to_string(),
                    ok(obs_body(&[
                        ("2020-01-02", "3.1", "2026-09-05", "9999-12-31"),
                        ("2020-01-09", "3.2", "2026-09-05", "9999-12-31"),
                    ])),
                ),
            ]),
        };
        let file = SeriesFile {
            series: vec![
                entry("HOUST", "national", true),
                entry("MORTGAGE30US", "national", true),
                entry("BPPRIV055133", "county", false),
            ],
        };
        let result = load_fred_series(&mut conn, &file, &client).expect("load");
        assert!(result.assertion_report.contains("S1 HOUST expected>=1 actual=3"));
        assert!(result.assertion_report.contains("S4 HOUST expected=printed_not_failed actual_null_count=1"));
        assert!(result.profile_report.contains("cadence_gaps(missing_months)=2020-02"));
        assert!(result.profile_report.contains("null_dates=2020-03-01"));
        assert!(result
            .profile_report
            .contains("BPPRIV055133 is absent"));
        assert!(result.vintage_report.contains("varies_within_series=false"));
        let stored_null: Option<f64> = conn
            .query_row(
                "SELECT value FROM series_observations
                 WHERE series_id = 'HOUST' AND observation_date = '2020-03-01'",
                [],
                |row| row.get(0),
            )
            .expect("null row");
        assert_eq!(stored_null, None);
        let source: String = conn
            .query_row(
                "SELECT source_id FROM load_runs WHERE id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .expect("run");
        assert_eq!(source, FRED_SOURCE_ID);
    }

    #[test]
    fn empty_series_fails_and_is_named() {
        let mut conn = open();
        let client = MapClient {
            meta: BTreeMap::from([(
                "HOUST".to_string(),
                ok(meta_body("HOUST", "Monthly")),
            )]),
            obs: BTreeMap::from([(
                "HOUST".to_string(),
                    FredHttpResponse {
                        status: 200,
                        body: r#"{"count":0,"observations":[]}"#.to_string(),
                        url_redacted: String::new(),
                    },
            )]),
        };
        let file = SeriesFile {
            series: vec![entry("HOUST", "national", true)],
        };
        match load_fred_series(&mut conn, &file, &client) {
            Err(StoreError::FredAssertion { code, message }) => {
                assert_eq!(code, "S1");
                assert!(message.contains("HOUST"));
            }
            other => panic!("expected S1, got {other:?}"),
        }
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM load_runs", [], |row| row.get(0))
            .expect("runs");
        assert_eq!(runs, 0);
    }

    #[test]
    fn unparsable_value_fails_without_coercion() {
        let mut conn = open();
        let client = MapClient {
            meta: BTreeMap::from([(
                "HOUST".to_string(),
                ok(meta_body("HOUST", "Monthly")),
            )]),
            obs: BTreeMap::from([(
                "HOUST".to_string(),
                    FredHttpResponse {
                        status: 200,
                        body: obs_body(&[("2020-01-01", "not-a-number", "2026-09-05", "9999-12-31")]),
                        url_redacted: String::new(),
                    },
            )]),
        };
        let file = SeriesFile {
            series: vec![entry("HOUST", "national", true)],
        };
        match load_fred_series(&mut conn, &file, &client) {
            Err(StoreError::FredAssertion { code, message }) => {
                assert_eq!(code, "S5");
                assert!(message.contains("not-a-number"));
            }
            other => panic!("expected S5, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_primary_key_fails() {
        let mut conn = open();
        let client = MapClient {
            meta: BTreeMap::from([(
                "HOUST".to_string(),
                ok(meta_body("HOUST", "Monthly")),
            )]),
            obs: BTreeMap::from([(
                "HOUST".to_string(),
                    FredHttpResponse {
                        status: 200,
                        body: obs_body(&[
                            ("2020-01-01", "1", "2026-09-05", "9999-12-31"),
                            ("2020-01-01", "2", "2026-09-05", "9999-12-31"),
                        ]),
                        url_redacted: String::new(),
                    },
            )]),
        };
        let file = SeriesFile {
            series: vec![entry("HOUST", "national", true)],
        };
        match load_fred_series(&mut conn, &file, &client) {
            Err(StoreError::FredAssertion { code, message }) => {
                assert_eq!(code, "S3");
                assert!(message.contains("HOUST"));
            }
            other => panic!("expected S3, got {other:?}"),
        }
    }
}
