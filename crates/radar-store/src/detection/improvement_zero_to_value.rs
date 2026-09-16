use std::collections::BTreeMap;

use radar_config::RadarConfig;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::list::DEFAULT_PAGE_LIMIT;
use crate::normalise::normalise_owner_name;
use crate::StoreError;

pub const SIGNAL_KEY: &str = "improvement_zero_to_value";

// This detects a value changing between two fetches. It does not
// observe construction. A parcel can move from zero to a value
// because a building was completed, because the assessor revalued
// it, or because the county corrected a record.
//
// The window is the gap between the two published dates, not the
// date of the work. A 205 day window means the change happened
// somewhere inside 205 days.
//
// Waukesha run 5 read seven municipalities from a January layer and
// the rest from August, so windows differ by municipality within one
// run pair.
//
// Run 14 and run 16 both published on 2026-08-23. Comparing them
// produced zero transitions because the roll had not republished.
// That is not the same as observing no change. If both sides share
// one published date, the pair is not comparable. If a side carries
// no value_as_of at all, as on every Milwaukee row, the values are
// not dated; that is not a window of zero.

#[derive(Debug, Clone, PartialEq)]
pub struct DetectionBreakdown {
    pub earlier_load_run_id: i64,
    pub later_load_run_id: i64,
    pub keys_in_both: i64,
    pub keys_earlier_only: i64,
    pub keys_later_only: i64,
    pub qualifying_transitions: i64,
    pub earlier_null_later_value: i64,
    pub earlier_value_later_null: i64,
    pub both_null: i64,
    pub both_zero: i64,
    pub other_numeric: i64,
}

impl DetectionBreakdown {
    pub fn null_either(&self) -> i64 {
        self.earlier_null_later_value + self.earlier_value_later_null + self.both_null
    }

    pub fn four_categories_sum(&self) -> i64 {
        self.qualifying_transitions + self.null_either() + self.both_zero + self.other_numeric
    }

    pub fn accounts_for_every_key_in_both(&self) -> bool {
        self.four_categories_sum() == self.keys_in_both
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub county_fips: Option<String>,
    pub muni_code: Option<String>,
    pub taxkey: String,
    pub earlier_value: f64,
    pub later_value: f64,
    pub earlier_value_as_of: Option<String>,
    pub later_value_as_of: Option<String>,
    pub window_days: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Comparability {
    Comparable,
    SamePublishedDate { date: String },
    ValuesNotDated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectionOutcome {
    pub comparability: Comparability,
    pub earlier_value_as_of: BTreeMap<Option<String>, i64>,
    pub later_value_as_of: BTreeMap<Option<String>, i64>,
    pub breakdown: DetectionBreakdown,
    pub observations: Vec<Observation>,
    pub window_days_distribution: BTreeMap<Option<i64>, i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QualifyingSampleRow {
    pub taxkey: String,
    pub municipality: Option<String>,
    pub earlier_owner_name: Option<String>,
    pub later_owner_name: Option<String>,
    pub earlier_value: f64,
    pub later_value: f64,
    pub earlier_value_as_of: Option<String>,
    pub later_value_as_of: Option<String>,
    pub window_days: Option<i64>,
    pub later_assessed_value: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DetectionListQuery {
    // Municipality, owner_type and name_contains apply at parcel level.
    // Band, min_parcels and min_improvement_value_zero are owner-group
    // aggregates and do not apply here.
    pub municipality: Option<String>,
    pub owner_type: Option<String>,
    pub name_contains: Option<String>,
    pub min_window_days: Option<i64>,
    pub max_window_days: Option<i64>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DetectionListRow {
    pub id: i64,
    pub signal_key: String,
    pub detected_at: String,
    pub earlier_load_run_id: i64,
    pub later_load_run_id: i64,
    pub county_fips: Option<String>,
    pub muni_code: Option<String>,
    pub municipality: Option<String>,
    pub taxkey: String,
    pub owner_name: Option<String>,
    pub owner_type: Option<String>,
    pub earlier_value: Option<f64>,
    pub later_value: Option<f64>,
    pub earlier_value_as_of: Option<String>,
    pub later_value_as_of: Option<String>,
    pub window_days: Option<i64>,
    pub config_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DetectionListPage {
    pub rows: Vec<DetectionListRow>,
    pub total_after_filter: i64,
    pub offset: i64,
    pub limit: i64,
    pub detected_at: Option<String>,
    pub county_fips: String,
}

pub fn detect_improvement_zero_to_value(
    conn: &Connection,
    earlier_load_run_id: i64,
    later_load_run_id: i64,
) -> Result<DetectionOutcome, StoreError> {
    let earlier_value_as_of = value_as_of_distribution(conn, earlier_load_run_id)?;
    let later_value_as_of = value_as_of_distribution(conn, later_load_run_id)?;
    let comparability = pair_comparability(&earlier_value_as_of, &later_value_as_of);
    let keys_earlier_only = count_only_in(conn, earlier_load_run_id, later_load_run_id)?;
    let keys_later_only = count_only_in(conn, later_load_run_id, earlier_load_run_id)?;
    let keys_in_both = count_in_both(conn, earlier_load_run_id, later_load_run_id)?;

    if comparability != Comparability::Comparable {
        return Ok(DetectionOutcome {
            comparability,
            earlier_value_as_of,
            later_value_as_of,
            breakdown: DetectionBreakdown {
                earlier_load_run_id,
                later_load_run_id,
                keys_in_both,
                keys_earlier_only,
                keys_later_only,
                qualifying_transitions: 0,
                earlier_null_later_value: 0,
                earlier_value_later_null: 0,
                both_null: 0,
                both_zero: 0,
                other_numeric: 0,
            },
            observations: Vec::new(),
            window_days_distribution: BTreeMap::new(),
        });
    }

    let mut stmt = conn.prepare(
        "SELECT e.county_fips, e.muni_code, e.taxkey,
                e.improvement_value, l.improvement_value,
                e.value_as_of, l.value_as_of
         FROM entities e
         INNER JOIN entities l
           ON l.load_run_id = ?2
          AND l.county_fips IS e.county_fips
          AND l.muni_code IS e.muni_code
          AND l.taxkey = e.taxkey
         WHERE e.load_run_id = ?1",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![earlier_load_run_id, later_load_run_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        },
    )?;

    let mut classified = 0_i64;
    let mut qualifying_transitions = 0_i64;
    let mut earlier_null_later_value = 0_i64;
    let mut earlier_value_later_null = 0_i64;
    let mut both_null = 0_i64;
    let mut both_zero = 0_i64;
    let mut other_numeric = 0_i64;
    let mut observations = Vec::new();
    let mut window_days_distribution: BTreeMap<Option<i64>, i64> = BTreeMap::new();

    for row in rows {
        let (
            county_fips,
            muni_code,
            taxkey,
            earlier_value,
            later_value,
            earlier_value_as_of,
            later_value_as_of,
        ) = row?;
        classified += 1;
        match (earlier_value, later_value) {
            (None, None) => both_null += 1,
            (None, Some(_)) => earlier_null_later_value += 1,
            (Some(_), None) => earlier_value_later_null += 1,
            (Some(earlier), Some(later)) if earlier == 0.0 && later > 0.0 => {
                qualifying_transitions += 1;
                let window = window_days(conn, earlier_value_as_of.as_deref(), later_value_as_of.as_deref())?;
                *window_days_distribution.entry(window).or_insert(0) += 1;
                observations.push(Observation {
                    county_fips,
                    muni_code,
                    taxkey,
                    earlier_value: earlier,
                    later_value: later,
                    earlier_value_as_of,
                    later_value_as_of,
                    window_days: window,
                });
            }
            (Some(earlier), Some(later)) if earlier == 0.0 && later == 0.0 => {
                both_zero += 1;
            }
            (Some(_), Some(_)) => other_numeric += 1,
        }
    }

    debug_assert_eq!(classified, keys_in_both);
    Ok(DetectionOutcome {
        comparability: Comparability::Comparable,
        earlier_value_as_of,
        later_value_as_of,
        breakdown: DetectionBreakdown {
            earlier_load_run_id,
            later_load_run_id,
            keys_in_both,
            keys_earlier_only,
            keys_later_only,
            qualifying_transitions,
            earlier_null_later_value,
            earlier_value_later_null,
            both_null,
            both_zero,
            other_numeric,
        },
        observations,
        window_days_distribution,
    })
}

pub fn insert_detections(
    conn: &mut Connection,
    earlier_load_run_id: i64,
    later_load_run_id: i64,
    observations: &[Observation],
    config_fingerprint: &str,
    detected_at: &str,
) -> Result<i64, StoreError> {
    let tx = conn.transaction()?;
    let mut inserted = 0_i64;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO detections (
                 signal_key, detected_at, earlier_load_run_id, later_load_run_id,
                 county_fips, muni_code, taxkey, earlier_value, later_value,
                 earlier_value_as_of, later_value_as_of, window_days, config_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )?;
        for row in observations {
            stmt.execute(rusqlite::params![
                SIGNAL_KEY,
                detected_at,
                earlier_load_run_id,
                later_load_run_id,
                row.county_fips,
                row.muni_code,
                row.taxkey,
                row.earlier_value,
                row.later_value,
                row.earlier_value_as_of,
                row.later_value_as_of,
                row.window_days,
                config_fingerprint,
            ])?;
            inserted += 1;
        }
    }
    tx.commit()?;
    Ok(inserted)
}

pub fn now_detected_at(conn: &Connection) -> Result<String, StoreError> {
    let stamp: String =
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })?;
    Ok(stamp)
}

pub fn newest_set_for_pair(
    conn: &Connection,
    earlier_load_run_id: i64,
    later_load_run_id: i64,
) -> Result<Option<String>, StoreError> {
    let stamp: Option<String> = conn.query_row(
        "SELECT MAX(detected_at)
         FROM detections
         WHERE signal_key = ?1
           AND earlier_load_run_id = ?2
           AND later_load_run_id = ?3",
        rusqlite::params![SIGNAL_KEY, earlier_load_run_id, later_load_run_id],
        |row| row.get(0),
    )?;
    Ok(stamp)
}

pub fn sample_qualifying(
    conn: &Connection,
    earlier_load_run_id: i64,
    later_load_run_id: i64,
    limit: i64,
) -> Result<Vec<QualifyingSampleRow>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT e.taxkey,
                m.muni_name,
                e.owner_name,
                l.owner_name,
                e.improvement_value,
                l.improvement_value,
                e.value_as_of,
                l.value_as_of,
                l.assessed_value,
                CASE
                    WHEN e.value_as_of IS NULL OR l.value_as_of IS NULL THEN NULL
                    ELSE CAST(ABS(julianday(l.value_as_of) - julianday(e.value_as_of)) AS INTEGER)
                END
         FROM entities e
         INNER JOIN entities l
           ON l.load_run_id = ?2
          AND l.county_fips IS e.county_fips
          AND l.muni_code IS e.muni_code
          AND l.taxkey = e.taxkey
         LEFT JOIN muni_lookup m
           ON m.county_fips IS e.county_fips
          AND m.muni_code IS e.muni_code
         WHERE e.load_run_id = ?1
           AND e.improvement_value = 0
           AND l.improvement_value > 0
         ORDER BY RANDOM()
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![earlier_load_run_id, later_load_run_id, limit],
        |row| {
            Ok(QualifyingSampleRow {
                taxkey: row.get(0)?,
                municipality: row.get(1)?,
                earlier_owner_name: row.get(2)?,
                later_owner_name: row.get(3)?,
                earlier_value: row.get(4)?,
                later_value: row.get(5)?,
                earlier_value_as_of: row.get(6)?,
                later_value_as_of: row.get(7)?,
                later_assessed_value: row.get(8)?,
                window_days: row.get(9)?,
            })
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn print_breakdown(outcome: &DetectionOutcome) {
    let b = &outcome.breakdown;
    println!(
        "pair earlier={} later={}",
        b.earlier_load_run_id, b.later_load_run_id
    );
    print_value_as_of_side("earlier", &outcome.earlier_value_as_of);
    print_value_as_of_side("later", &outcome.later_value_as_of);
    match &outcome.comparability {
        Comparability::SamePublishedDate { date } => {
            println!(
                "pair not comparable, both sides published on the same date, {date}"
            );
            println!("keys present in both runs: {}", b.keys_in_both);
            println!("keys in the earlier only: {}", b.keys_earlier_only);
            println!("keys in the later only: {}", b.keys_later_only);
            println!("transition categories not computed; nothing could have changed");
            return;
        }
        Comparability::ValuesNotDated => {
            println!("values not dated on this source");
            println!("keys present in both runs: {}", b.keys_in_both);
            println!("keys in the earlier only: {}", b.keys_earlier_only);
            println!("keys in the later only: {}", b.keys_later_only);
            println!("transition categories not computed; windows would be unknown, not zero");
            return;
        }
        Comparability::Comparable => {}
    }
    println!("keys present in both runs: {}", b.keys_in_both);
    println!("keys in the earlier only: {}", b.keys_earlier_only);
    println!("keys in the later only: {}", b.keys_later_only);
    println!("qualifying transitions: {}", b.qualifying_transitions);
    println!(
        "earlier null and later has a value: {}",
        b.earlier_null_later_value
    );
    println!(
        "earlier has a value and later null: {}",
        b.earlier_value_later_null
    );
    println!("both null: {}", b.both_null);
    println!("earlier 0 and later 0: {}", b.both_zero);
    println!(
        "other both-present numeric (not 0-to-value, not 0-to-0): {}",
        b.other_numeric
    );
    println!(
        "four categories (qualifying, null either, 0-to-0, other numeric) sum={} keys_in_both={} accounts={}",
        b.four_categories_sum(),
        b.keys_in_both,
        b.accounts_for_every_key_in_both()
    );
    println!("window_days distribution (qualifying rows):");
    for (window, count) in &outcome.window_days_distribution {
        match window {
            None => println!("  null: {count}"),
            Some(days) => println!("  {days}: {count}"),
        }
    }
}

pub fn list_detections(
    config: &RadarConfig,
    conn: &Connection,
    query: &DetectionListQuery,
) -> Result<DetectionListPage, StoreError> {
    let county_fips = config.manifest.county_fips.as_str();
    let offset = query.offset.unwrap_or(0).max(0);
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_LIMIT).max(1);
    let newest: Option<String> = conn.query_row(
        "SELECT MAX(detected_at)
         FROM detections
         WHERE signal_key = ?1
           AND county_fips = ?2",
        rusqlite::params![SIGNAL_KEY, county_fips],
        |row| row.get(0),
    )?;
    let Some(detected_at) = newest else {
        return Ok(DetectionListPage {
            rows: Vec::new(),
            total_after_filter: 0,
            offset,
            limit,
            detected_at: None,
            county_fips: county_fips.to_string(),
        });
    };

    let mut stmt = conn.prepare(
        "SELECT d.id, d.signal_key, d.detected_at, d.earlier_load_run_id, d.later_load_run_id,
                d.county_fips, d.muni_code, m.muni_name, d.taxkey,
                later.owner_name, later.owner_type,
                d.earlier_value, d.later_value,
                d.earlier_value_as_of, d.later_value_as_of,
                d.window_days, d.config_fingerprint
         FROM detections d
         INNER JOIN entities later
           ON later.load_run_id = d.later_load_run_id
          AND later.county_fips IS d.county_fips
          AND later.muni_code IS d.muni_code
          AND later.taxkey = d.taxkey
         LEFT JOIN muni_lookup m
           ON m.county_fips IS d.county_fips
          AND m.muni_code IS d.muni_code
         WHERE d.signal_key = ?1
           AND d.county_fips = ?2
           AND d.detected_at = ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![SIGNAL_KEY, county_fips, detected_at],
        |row| {
            Ok(DetectionListRow {
                id: row.get(0)?,
                signal_key: row.get(1)?,
                detected_at: row.get(2)?,
                earlier_load_run_id: row.get(3)?,
                later_load_run_id: row.get(4)?,
                county_fips: row.get(5)?,
                muni_code: row.get(6)?,
                municipality: row.get(7)?,
                taxkey: row.get(8)?,
                owner_name: row.get(9)?,
                owner_type: row.get(10)?,
                earlier_value: row.get(11)?,
                later_value: row.get(12)?,
                earlier_value_as_of: row.get(13)?,
                later_value_as_of: row.get(14)?,
                window_days: row.get(15)?,
                config_fingerprint: row.get(16)?,
            })
        },
    )?;
    let mut matched = Vec::new();
    for row in rows {
        let row = row?;
        if !row_matches(query, &row) {
            continue;
        }
        matched.push(row);
    }
    let total_after_filter = matched.len() as i64;
    let end = (offset + limit).min(total_after_filter);
    let start = offset.min(total_after_filter);
    let page = if start >= total_after_filter {
        Vec::new()
    } else {
        matched[start as usize..end as usize].to_vec()
    };
    Ok(DetectionListPage {
        rows: page,
        total_after_filter,
        offset,
        limit,
        detected_at: Some(detected_at),
        county_fips: county_fips.to_string(),
    })
}

fn row_matches(query: &DetectionListQuery, row: &DetectionListRow) -> bool {
    if let Some(muni) = nonempty(query.municipality.as_deref()) {
        if row.muni_code.as_deref() != Some(muni) {
            return false;
        }
    }
    if let Some(owner_type) = nonempty(query.owner_type.as_deref()) {
        if row.owner_type.as_deref() != Some(owner_type) {
            return false;
        }
    }
    if let Some(needle) = nonempty(query.name_contains.as_deref()) {
        let normalised = normalise_owner_name(needle);
        let hay = row
            .owner_name
            .as_deref()
            .map(normalise_owner_name)
            .unwrap_or_default();
        if normalised.is_empty() || !hay.contains(&normalised) {
            return false;
        }
    }
    if let Some(min) = query.min_window_days {
        match row.window_days {
            Some(days) if days >= min => {}
            _ => return false,
        }
    }
    if let Some(max) = query.max_window_days {
        match row.window_days {
            Some(days) if days <= max => {}
            _ => return false,
        }
    }
    true
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn count_only_in(
    conn: &Connection,
    this_run: i64,
    other_run: i64,
) -> Result<i64, StoreError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM entities e
         WHERE e.load_run_id = ?1
           AND NOT EXISTS (
               SELECT 1
               FROM entities o
               WHERE o.load_run_id = ?2
                 AND o.county_fips IS e.county_fips
                 AND o.muni_code IS e.muni_code
                 AND o.taxkey = e.taxkey
           )",
        rusqlite::params![this_run, other_run],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn count_in_both(
    conn: &Connection,
    earlier_load_run_id: i64,
    later_load_run_id: i64,
) -> Result<i64, StoreError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM entities e
         INNER JOIN entities l
           ON l.load_run_id = ?2
          AND l.county_fips IS e.county_fips
          AND l.muni_code IS e.muni_code
          AND l.taxkey = e.taxkey
         WHERE e.load_run_id = ?1",
        rusqlite::params![earlier_load_run_id, later_load_run_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn dated_values(dist: &BTreeMap<Option<String>, i64>) -> Vec<&str> {
    dist.iter()
        .filter_map(|(date, count)| match (date, *count) {
            (Some(date), n) if n > 0 => Some(date.as_str()),
            _ => None,
        })
        .collect()
}

fn pair_comparability(
    earlier: &BTreeMap<Option<String>, i64>,
    later: &BTreeMap<Option<String>, i64>,
) -> Comparability {
    let earlier_dates = dated_values(earlier);
    let later_dates = dated_values(later);
    if earlier_dates.is_empty() || later_dates.is_empty() {
        return Comparability::ValuesNotDated;
    }
    if earlier_dates.len() == 1 && later_dates.len() == 1 && earlier_dates[0] == later_dates[0] {
        return Comparability::SamePublishedDate {
            date: earlier_dates[0].to_string(),
        };
    }
    Comparability::Comparable
}

fn value_as_of_distribution(
    conn: &Connection,
    load_run_id: i64,
) -> Result<BTreeMap<Option<String>, i64>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT value_as_of, COUNT(*)
         FROM entities
         WHERE load_run_id = ?1
         GROUP BY value_as_of",
    )?;
    let rows = stmt.query_map([load_run_id], |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (date, count) = row?;
        out.insert(date, count);
    }
    Ok(out)
}

fn print_value_as_of_side(label: &str, dist: &BTreeMap<Option<String>, i64>) {
    println!("value_as_of on {label}:");
    if dist.is_empty() {
        println!("  (no rows)");
        return;
    }
    for (date, count) in dist {
        match date {
            None => println!("  null: {count}"),
            Some(date) => println!("  {date}: {count}"),
        }
    }
}

fn window_days(
    conn: &Connection,
    earlier: Option<&str>,
    later: Option<&str>,
) -> Result<Option<i64>, StoreError> {
    let earlier = nonempty(earlier);
    let later = nonempty(later);
    if earlier.is_none() || later.is_none() {
        return Ok(None);
    }
    let days: Option<i64> = conn.query_row(
        "SELECT CAST(ABS(julianday(?1) - julianday(?2)) AS INTEGER)",
        rusqlite::params![later, earlier],
        |row| row.get(0),
    )?;
    Ok(days)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        detect_improvement_zero_to_value, insert_detections, list_detections, newest_set_for_pair,
        window_days, Comparability, DetectionListQuery, SIGNAL_KEY,
    };
    use crate::migrate::migrate;
    use crate::rank::tests::{test_bands, test_config};

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (1, 'src-test', 'success', '2026-01-01'),
                    (2, 'src-test', 'success', '2026-08-01')",
            [],
        )
        .expect("runs");
        conn.execute(
            "INSERT INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
             VALUES ('55133', 'WAKC', 'Waukesha City', 'test', 1)",
            [],
        )
        .expect("muni");
        conn
    }

    fn add(
        conn: &Connection,
        run: i64,
        taxkey: &str,
        improvement: Option<f64>,
        value_as_of: Option<&str>,
        owner_name: &str,
        owner_type: &str,
    ) {
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, taxkey, county_fips, muni_code, improvement_value,
                 value_as_of, owner_name, owner_type, assessed_value
             ) VALUES (?1, ?2, '55133', 'WAKC', ?3, ?4, ?5, ?6, 100)",
            rusqlite::params![run, taxkey, improvement, value_as_of, owner_name, owner_type],
        )
        .expect("entity");
    }

    #[test]
    fn zero_to_a_value_is_a_transition() {
        let conn = open();
        add(&conn, 1, "A", Some(0.0), Some("2026-01-01"), "ACME LLC", "entity");
        add(&conn, 2, "A", Some(250000.0), Some("2026-08-01"), "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.breakdown.qualifying_transitions, 1);
        assert_eq!(outcome.observations.len(), 1);
        assert_eq!(outcome.observations[0].taxkey, "A");
        assert_eq!(outcome.observations[0].window_days, Some(212));
    }

    #[test]
    fn null_to_a_value_is_not_and_is_counted_separately() {
        let conn = open();
        add(&conn, 1, "A", None, Some("2026-01-01"), "ACME LLC", "entity");
        add(&conn, 2, "A", Some(10.0), Some("2026-08-01"), "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.breakdown.qualifying_transitions, 0);
        assert_eq!(outcome.breakdown.earlier_null_later_value, 1);
        assert!(outcome.observations.is_empty());
    }

    #[test]
    fn a_value_to_null_is_not_and_is_counted_separately() {
        let conn = open();
        add(&conn, 1, "A", Some(10.0), Some("2026-01-01"), "ACME LLC", "entity");
        add(&conn, 2, "A", None, Some("2026-08-01"), "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.breakdown.qualifying_transitions, 0);
        assert_eq!(outcome.breakdown.earlier_value_later_null, 1);
    }

    #[test]
    fn zero_to_zero_is_not() {
        let conn = open();
        add(&conn, 1, "A", Some(0.0), Some("2026-01-01"), "ACME LLC", "entity");
        add(&conn, 2, "A", Some(0.0), Some("2026-08-01"), "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.breakdown.qualifying_transitions, 0);
        assert_eq!(outcome.breakdown.both_zero, 1);
    }

    #[test]
    fn key_absent_from_earlier_run_is_not() {
        let conn = open();
        add(&conn, 1, "KEEP", Some(1.0), Some("2026-01-01"), "KEEP LLC", "entity");
        add(&conn, 2, "KEEP", Some(1.0), Some("2026-08-01"), "KEEP LLC", "entity");
        add(&conn, 2, "A", Some(10.0), Some("2026-08-01"), "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.breakdown.qualifying_transitions, 0);
        assert_eq!(outcome.breakdown.keys_later_only, 1);
        assert_eq!(outcome.breakdown.keys_in_both, 1);
        assert_eq!(outcome.comparability, Comparability::Comparable);
    }

    #[test]
    fn window_days_is_null_when_either_date_is_null_never_zero() {
        let conn = open();
        assert_eq!(window_days(&conn, None, Some("2026-08-01")).unwrap(), None);
        assert_eq!(window_days(&conn, Some("2026-01-01"), None).unwrap(), None);
        assert_eq!(window_days(&conn, None, None).unwrap(), None);
        assert_eq!(
            window_days(&conn, Some("2026-01-01"), Some("2026-01-01")).unwrap(),
            Some(0)
        );
        add(&conn, 1, "KEEP", Some(1.0), Some("2026-01-01"), "KEEP LLC", "entity");
        add(&conn, 2, "KEEP", Some(1.0), Some("2026-08-01"), "KEEP LLC", "entity");
        add(&conn, 1, "A", Some(0.0), None, "ACME LLC", "entity");
        add(&conn, 2, "A", Some(5.0), None, "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.comparability, Comparability::Comparable);
        assert_eq!(outcome.observations[0].window_days, None);
        assert_eq!(outcome.window_days_distribution.get(&None).copied(), Some(1));
    }

    #[test]
    fn four_categories_account_for_every_key_present_in_both() {
        let conn = open();
        add(&conn, 1, "T", Some(0.0), Some("2026-01-01"), "T", "entity");
        add(&conn, 2, "T", Some(9.0), Some("2026-08-01"), "T", "entity");
        add(&conn, 1, "N", None, Some("2026-01-01"), "N", "entity");
        add(&conn, 2, "N", Some(1.0), Some("2026-08-01"), "N", "entity");
        add(&conn, 1, "V", Some(3.0), Some("2026-01-01"), "V", "entity");
        add(&conn, 2, "V", None, Some("2026-08-01"), "V", "entity");
        add(&conn, 1, "Z", Some(0.0), Some("2026-01-01"), "Z", "entity");
        add(&conn, 2, "Z", Some(0.0), Some("2026-08-01"), "Z", "entity");
        add(&conn, 1, "O", Some(4.0), Some("2026-01-01"), "O", "entity");
        add(&conn, 2, "O", Some(8.0), Some("2026-08-01"), "O", "entity");
        add(&conn, 1, "B", None, None, "B", "entity");
        add(&conn, 2, "B", None, None, "B", "entity");
        add(&conn, 1, "X", Some(0.0), Some("2026-01-01"), "X", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.breakdown.keys_in_both, 6);
        assert_eq!(outcome.breakdown.keys_earlier_only, 1);
        assert_eq!(outcome.breakdown.qualifying_transitions, 1);
        assert_eq!(outcome.breakdown.earlier_null_later_value, 1);
        assert_eq!(outcome.breakdown.earlier_value_later_null, 1);
        assert_eq!(outcome.breakdown.both_zero, 1);
        assert_eq!(outcome.breakdown.other_numeric, 1);
        assert_eq!(outcome.breakdown.both_null, 1);
        assert!(outcome.breakdown.accounts_for_every_key_in_both());
    }

    #[test]
    fn second_run_against_the_same_pair_inserts_rather_than_replaces() {
        let mut conn = open();
        add(&conn, 1, "A", Some(0.0), Some("2026-01-01"), "ACME LLC", "entity");
        add(&conn, 2, "A", Some(12.0), Some("2026-08-01"), "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        insert_detections(&mut conn, 1, 2, &outcome.observations, "fp-1", "2026-09-04T01:00:00.000Z")
            .expect("first insert");
        insert_detections(&mut conn, 1, 2, &outcome.observations, "fp-1", "2026-09-04T02:00:00.000Z")
            .expect("second insert");
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))
            .expect("count");
        assert_eq!(total, 2);
        let newest = newest_set_for_pair(&conn, 1, 2).expect("newest").expect("stamp");
        assert_eq!(newest, "2026-09-04T02:00:00.000Z");
        let newest_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections
                 WHERE signal_key = ?1 AND earlier_load_run_id = 1 AND later_load_run_id = 2
                   AND detected_at = ?2",
                rusqlite::params![SIGNAL_KEY, newest],
                |row| row.get(0),
            )
            .expect("newest count");
        assert_eq!(newest_count, 1);
    }

    #[test]
    fn list_returns_newest_set_for_loaded_county_with_window_filter() {
        let mut conn = open();
        add(&conn, 1, "A", Some(0.0), Some("2026-01-01"), "ACME LLC", "entity");
        add(&conn, 2, "A", Some(12.0), Some("2026-08-01"), "ACME LLC", "entity");
        add(&conn, 1, "B", Some(0.0), None, "BETA LLC", "entity");
        add(&conn, 2, "B", Some(9.0), None, "BETA LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        insert_detections(&mut conn, 1, 2, &outcome.observations, "fp-1", "2026-09-04T03:00:00.000Z")
            .expect("insert");
        let config = test_config(1, "caveat", test_bands());
        let page = list_detections(
            &config,
            &conn,
            &DetectionListQuery {
                min_window_days: Some(1),
                ..DetectionListQuery::default()
            },
        )
        .expect("list");
        assert_eq!(page.total_after_filter, 1);
        assert_eq!(page.rows[0].taxkey, "A");
        assert_eq!(page.rows[0].window_days, Some(212));
        let null_windows = list_detections(&config, &conn, &DetectionListQuery::default())
            .expect("all");
        assert_eq!(null_windows.total_after_filter, 2);
    }

    #[test]
    fn same_single_published_date_is_not_comparable() {
        let conn = open();
        add(&conn, 1, "A", Some(0.0), Some("2026-08-23"), "ACME LLC", "entity");
        add(&conn, 2, "A", Some(400000.0), Some("2026-08-23"), "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(
            outcome.comparability,
            Comparability::SamePublishedDate {
                date: "2026-08-23".to_string()
            }
        );
        assert!(outcome.observations.is_empty());
        assert_eq!(outcome.breakdown.keys_in_both, 1);
    }

    #[test]
    fn undated_side_is_values_not_dated_not_a_zero_window() {
        let conn = open();
        add(&conn, 1, "A", Some(0.0), None, "ACME LLC", "entity");
        add(&conn, 2, "A", Some(9.0), None, "ACME LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.comparability, Comparability::ValuesNotDated);
        assert!(outcome.observations.is_empty());
        assert_eq!(outcome.breakdown.keys_in_both, 1);
    }

    #[test]
    fn mixed_vintages_on_one_side_remain_comparable() {
        let conn = open();
        add(&conn, 1, "A", Some(0.0), Some("2026-08-10"), "ACME LLC", "entity");
        add(&conn, 1, "B", Some(0.0), Some("2025-12-31"), "BETA LLC", "entity");
        add(&conn, 2, "A", Some(12.0), Some("2026-08-23"), "ACME LLC", "entity");
        add(&conn, 2, "B", Some(0.0), Some("2026-08-23"), "BETA LLC", "entity");
        let outcome = detect_improvement_zero_to_value(&conn, 1, 2).expect("detect");
        assert_eq!(outcome.comparability, Comparability::Comparable);
        assert_eq!(outcome.breakdown.qualifying_transitions, 1);
        assert_eq!(outcome.observations[0].taxkey, "A");
    }
}
