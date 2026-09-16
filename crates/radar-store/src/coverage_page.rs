// Coverage page sections are fixed: geographies, attributes, and
// publication lag. coverage.toml supplies the stated entries and the
// store supplies the measured counts. This is not a page builder.
use std::collections::BTreeMap;

use radar_config::RadarConfig;
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::series::{days_between, today_ymd, FRED_SOURCE_ID};
use crate::StoreError;

pub const GAP_FRAMING: &str =
    "A gap is a stated fact about the source, not a fault in the tool.";

const ORIGIN_CONFIG: &str = "config";
const ORIGIN_MEASURED: &str = "measured";
const ORIGIN_NOT_STATED: &str = "not stated";
const VALUE_SOURCE_HISTORIC: &str = "historic_year_end";

const STORE_ATTRIBUTE_KEYS: &[&str] = &[
    "owner_name",
    "assessed_value",
    "improvement_value",
    "land_value",
    "property_class",
    "value_as_of",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MuniMeasured {
    pub muni_code: String,
    pub name: String,
    pub row_count: i64,
    pub value_as_of: Option<String>,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeographyRow {
    pub key: String,
    pub level: Option<String>,
    pub level_origin: String,
    pub note: Option<String>,
    pub note_origin: Option<String>,
    pub measured_row_count: i64,
    pub row_count_origin: String,
    pub measured_classed_count: Option<i64>,
    pub historic_municipalities: Vec<MuniMeasured>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttributeRow {
    pub key: String,
    pub stated_level: Option<String>,
    pub level_origin: String,
    pub note: Option<String>,
    pub note_origin: Option<String>,
    pub measured_null_count: Option<i64>,
    pub measured_row_count: Option<i64>,
    pub measured_null_percent: Option<f64>,
    pub measurement_origin: String,
    pub disagreement: bool,
    pub disagreement_text: Option<String>,
    pub not_in_store: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PublicationLagRow {
    pub source_id: String,
    pub display_name: Option<String>,
    pub series_id: Option<String>,
    pub stated_lag_days: Option<u32>,
    pub stated_measured: Option<bool>,
    pub stated_origin: String,
    pub newest_date: Option<String>,
    pub days_old: Option<i64>,
    pub age_text: String,
    pub measurement_origin: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoveragePage {
    pub framing: String,
    pub vertical_display_name: String,
    pub county_name: String,
    pub county_fips: String,
    pub config_version: String,
    pub geographies: Vec<GeographyRow>,
    pub attributes: Vec<AttributeRow>,
    pub publication_lags: Vec<PublicationLagRow>,
    pub three_value_null_count: i64,
    pub three_value_null_origin: String,
    pub value_null_municipalities: Vec<MuniMeasured>,
}

pub fn list_coverage(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<CoveragePage, StoreError> {
    let county_fips = config.entity.county_fips.as_str();
    let geographies = geography_rows(config, conn, county_fips)?;
    let attributes = attribute_rows(config, conn, county_fips)?;
    let publication_lags = publication_lag_rows(config, conn, county_fips)?;
    let three_value_null_count = three_value_nulls(conn, county_fips)?;
    let value_null_municipalities = value_null_munis(conn, county_fips)?;
    Ok(CoveragePage {
        framing: GAP_FRAMING.to_string(),
        vertical_display_name: config.manifest.vertical_display_name.clone(),
        county_name: config.manifest.county_name.clone(),
        county_fips: config.manifest.county_fips.clone(),
        config_version: config.manifest.config_version.clone(),
        geographies,
        attributes,
        publication_lags,
        three_value_null_count,
        three_value_null_origin: ORIGIN_MEASURED.to_string(),
        value_null_municipalities,
    })
}

fn geography_rows(
    config: &RadarConfig,
    conn: &Connection,
    loaded_county: &str,
) -> Result<Vec<GeographyRow>, StoreError> {
    let mut rows = Vec::new();
    for entry in &config.coverage.geographies {
        let (measured_row_count, measured_classed_count) =
            measure_geography(conn, &entry.key)?;
        let historic_municipalities = if geography_county_fips(&entry.key) == Some(loaded_county)
        {
            historic_munis(conn, loaded_county)?
        } else {
            Vec::new()
        };
        rows.push(GeographyRow {
            key: entry.key.clone(),
            level: Some(entry.level.as_label().to_string()),
            level_origin: ORIGIN_CONFIG.to_string(),
            note: Some(entry.note.clone()),
            note_origin: Some(ORIGIN_CONFIG.to_string()),
            measured_row_count,
            row_count_origin: ORIGIN_MEASURED.to_string(),
            measured_classed_count,
            historic_municipalities,
        });
    }
    Ok(rows)
}

fn measure_geography(
    conn: &Connection,
    key: &str,
) -> Result<(i64, Option<i64>), StoreError> {
    if let Some(fips) = geography_county_fips(key) {
        let count = count_where(conn, "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1", fips)?;
        return Ok((count, None));
    }
    let muni = key.to_uppercase();
    let count = count_where(
        conn,
        "SELECT COUNT(*) FROM current_parcels WHERE muni_code = ?1",
        &muni,
    )?;
    let classed = count_where(
        conn,
        "SELECT COUNT(*) FROM current_parcels
         WHERE muni_code = ?1
           AND prop_class IS NOT NULL
           AND TRIM(prop_class) != ''",
        &muni,
    )?;
    Ok((count, Some(classed)))
}

fn geography_county_fips(key: &str) -> Option<&'static str> {
    match key {
        "waukesha_county" => Some("55133"),
        "milwaukee_county" => Some("55079"),
        "washington_county" => Some("55131"),
        "ozaukee_county" => Some("55089"),
        _ => None,
    }
}

fn historic_munis(conn: &Connection, county_fips: &str) -> Result<Vec<MuniMeasured>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT p.muni_code,
                COALESCE(m.muni_name, p.muni_code),
                COUNT(*),
                MIN(p.value_as_of)
         FROM current_parcels p
         LEFT JOIN muni_lookup m
            ON m.county_fips = p.county_fips
           AND m.muni_code = p.muni_code
         WHERE p.county_fips = ?1
           AND p.value_source = ?2
         GROUP BY p.muni_code
         ORDER BY COALESCE(m.muni_name, p.muni_code), p.muni_code",
    )?;
    let rows = stmt.query_map(params![county_fips, VALUE_SOURCE_HISTORIC], |row| {
        Ok(MuniMeasured {
            muni_code: row.get(0)?,
            name: row.get(1)?,
            row_count: row.get(2)?,
            value_as_of: row.get(3)?,
            origin: ORIGIN_MEASURED.to_string(),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn attribute_rows(
    config: &RadarConfig,
    conn: &Connection,
    county_fips: &str,
) -> Result<Vec<AttributeRow>, StoreError> {
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in &config.coverage.attributes {
        seen.insert(canonical_attribute_key(&entry.key));
        rows.push(attribute_from_config(conn, county_fips, entry)?);
    }
    for key in STORE_ATTRIBUTE_KEYS {
        if seen.insert(canonical_attribute_key(key)) {
            rows.push(store_only_attribute(conn, county_fips, key)?);
        }
    }
    Ok(rows)
}

fn canonical_attribute_key(key: &str) -> String {
    match key {
        "prop_class" => "property_class".to_string(),
        other => other.to_string(),
    }
}

fn attribute_from_config(
    conn: &Connection,
    county_fips: &str,
    entry: &radar_config::CoverageEntry,
) -> Result<AttributeRow, StoreError> {
    let (null_count, total, not_in_store) = measure_attribute(conn, county_fips, &entry.key)?;
    let (percent, disagreement, disagreement_text) =
        compare_stated(entry.level.as_label(), null_count, total, not_in_store);
    Ok(AttributeRow {
        key: entry.key.clone(),
        stated_level: Some(entry.level.as_label().to_string()),
        level_origin: ORIGIN_CONFIG.to_string(),
        note: Some(entry.note.clone()),
        note_origin: Some(ORIGIN_CONFIG.to_string()),
        measured_null_count: null_count,
        measured_row_count: total,
        measured_null_percent: percent,
        measurement_origin: ORIGIN_MEASURED.to_string(),
        disagreement,
        disagreement_text,
        not_in_store,
    })
}

fn store_only_attribute(
    conn: &Connection,
    county_fips: &str,
    key: &str,
) -> Result<AttributeRow, StoreError> {
    let (null_count, total, not_in_store) = measure_attribute(conn, county_fips, key)?;
    Ok(AttributeRow {
        key: key.to_string(),
        stated_level: None,
        level_origin: ORIGIN_NOT_STATED.to_string(),
        note: None,
        note_origin: None,
        measured_null_count: null_count,
        measured_row_count: total,
        measured_null_percent: percent(null_count, total),
        measurement_origin: ORIGIN_MEASURED.to_string(),
        disagreement: false,
        disagreement_text: None,
        not_in_store,
    })
}

fn measure_attribute(
    conn: &Connection,
    county_fips: &str,
    key: &str,
) -> Result<(Option<i64>, Option<i64>, bool), StoreError> {
    let total = count_where(
        conn,
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1",
        county_fips,
    )?;
    match canonical_attribute_key(key).as_str() {
        "owner_name" => Ok((
            Some(nulls_in_column(conn, county_fips, "owner_name")?),
            Some(total),
            false,
        )),
        "assessed_value" => Ok((
            Some(nulls_in_column(conn, county_fips, "assessed_value")?),
            Some(total),
            false,
        )),
        "improvement_value" => Ok((
            Some(nulls_in_column(conn, county_fips, "improvement_value")?),
            Some(total),
            false,
        )),
        "land_value" => Ok((
            Some(nulls_in_column(conn, county_fips, "land_value")?),
            Some(total),
            false,
        )),
        "property_class" => Ok((
            Some(nulls_in_column(conn, county_fips, "prop_class")?),
            Some(total),
            false,
        )),
        "value_as_of" => Ok((
            Some(nulls_in_column(conn, county_fips, "value_as_of")?),
            Some(total),
            false,
        )),
        "historic_year_end_vintage" => {
            let historic = count_where_source(conn, county_fips, VALUE_SOURCE_HISTORIC)?;
            Ok((Some(total - historic), Some(total), false))
        }
        "municipality_lookup" => {
            let lookup = count_where(
                conn,
                "SELECT COUNT(*) FROM muni_lookup WHERE county_fips = ?1",
                county_fips,
            )?;
            Ok((Some(if lookup == 0 { total } else { 0 }), Some(total), false))
        }
        _ => Ok((None, None, true)),
    }
}

fn nulls_in_column(conn: &Connection, county_fips: &str, column: &str) -> Result<i64, StoreError> {
    // Column names are a closed set from STORE_ATTRIBUTE_KEYS / mappings above.
    let sql = format!(
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1 AND {column} IS NULL"
    );
    count_where(conn, &sql, county_fips)
}

fn count_where_source(
    conn: &Connection,
    county_fips: &str,
    source: &str,
) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1 AND value_source = ?2",
        params![county_fips, source],
        |row| row.get(0),
    )?)
}

fn compare_stated(
    stated: &str,
    null_count: Option<i64>,
    total: Option<i64>,
    not_in_store: bool,
) -> (Option<f64>, bool, Option<String>) {
    let pct = percent(null_count, total);
    if not_in_store {
        return (None, false, None);
    }
    let (Some(nulls), Some(rows)) = (null_count, total) else {
        return (pct, false, None);
    };
    if rows == 0 {
        return (pct, false, None);
    }
    let filled = rows - nulls;
    let disagreement = match stated {
        "full" => nulls > 0,
        "none" => filled > 0,
        "partial" => nulls == 0 || nulls == rows,
        _ => false,
    };
    let disagreement_text = if disagreement {
        Some(format!(
            "stated {stated}; measured {nulls} of {rows} null."
        ))
    } else {
        None
    };
    (pct, disagreement, disagreement_text)
}

fn percent(null_count: Option<i64>, total: Option<i64>) -> Option<f64> {
    match (null_count, total) {
        (Some(nulls), Some(rows)) if rows > 0 => Some((nulls as f64) * 100.0 / (rows as f64)),
        _ => None,
    }
}

fn publication_lag_rows(
    config: &RadarConfig,
    conn: &Connection,
    county_fips: &str,
) -> Result<Vec<PublicationLagRow>, StoreError> {
    let names: BTreeMap<String, String> = config
        .sources
        .sources
        .iter()
        .map(|source| (source.source_id.clone(), source.display_name.clone()))
        .collect();
    let mut rows = Vec::new();
    for lag in &config.coverage.publication_lags {
        if lag.source_id == FRED_SOURCE_ID {
            for series in config.series.series.iter().filter(|entry| entry.included) {
                rows.push(series_lag_row(conn, series, Some(lag), &names)?);
            }
        } else {
            rows.push(source_lag_row(conn, &lag.source_id, Some(lag), county_fips, &names)?);
        }
    }
    let fred_already = config
        .coverage
        .publication_lags
        .iter()
        .any(|lag| lag.source_id == FRED_SOURCE_ID);
    if !fred_already {
        for series in config
            .series
            .series
            .iter()
            .filter(|entry| entry.included && entry.geography == "county")
        {
            rows.push(series_lag_row(conn, series, None, &names)?);
        }
    }
    let spine = config.entity.spine_source_id.as_str();
    let spine_listed = config
        .coverage
        .publication_lags
        .iter()
        .any(|lag| lag.source_id == spine);
    if !spine_listed {
        rows.push(source_lag_row(conn, spine, None, county_fips, &names)?);
    }
    Ok(rows)
}

fn source_lag_row(
    conn: &Connection,
    source_id: &str,
    lag: Option<&radar_config::PublicationLag>,
    county_fips: &str,
    names: &BTreeMap<String, String>,
) -> Result<PublicationLagRow, StoreError> {
    let fips = source_county_fips(source_id).unwrap_or(county_fips);
    let newest: Option<String> = conn.query_row(
        "SELECT MAX(value_as_of) FROM current_parcels WHERE county_fips = ?1",
        [fips],
        |row| row.get(0),
    )?;
    let today = today_ymd();
    let days_old = newest.as_deref().and_then(|date| days_between(date, &today));
    let nulls = nulls_in_column(conn, fips, "value_as_of")?;
    let total = count_where(
        conn,
        "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1",
        fips,
    )?;
    let age_text = if total > 0 && nulls == total {
        "value_as_of is null on every row; age cannot be computed (measured).".to_string()
    } else if let Some(days) = days_old {
        format!("{days} days old (measured).")
    } else if newest.is_none() {
        "no dated rows in the store; age cannot be computed (measured).".to_string()
    } else {
        "age cannot be computed (measured).".to_string()
    };
    Ok(PublicationLagRow {
        source_id: source_id.to_string(),
        display_name: names.get(source_id).cloned(),
        series_id: None,
        stated_lag_days: lag.map(|entry| entry.lag_days),
        stated_measured: lag.map(|entry| entry.measured),
        stated_origin: if lag.is_some() {
            ORIGIN_CONFIG.to_string()
        } else {
            ORIGIN_NOT_STATED.to_string()
        },
        newest_date: newest,
        days_old,
        age_text,
        measurement_origin: ORIGIN_MEASURED.to_string(),
    })
}

fn series_lag_row(
    conn: &Connection,
    series: &radar_config::SeriesEntry,
    lag: Option<&radar_config::PublicationLag>,
    names: &BTreeMap<String, String>,
) -> Result<PublicationLagRow, StoreError> {
    let newest: Option<String> = conn.query_row(
        "SELECT MAX(observation_date) FROM series_observations WHERE series_id = ?1",
        [&series.series_id],
        |row| row.get(0),
    )?;
    let today = today_ymd();
    let days_old = newest.as_deref().and_then(|date| days_between(date, &today));
    let age_text = match days_old {
        Some(days) => format!("{days} days old (measured)."),
        None => "no observations in the store; age cannot be computed (measured).".to_string(),
    };
    Ok(PublicationLagRow {
        source_id: FRED_SOURCE_ID.to_string(),
        display_name: names.get(FRED_SOURCE_ID).cloned(),
        series_id: Some(series.series_id.clone()),
        stated_lag_days: lag.map(|entry| entry.lag_days),
        stated_measured: lag.map(|entry| entry.measured),
        stated_origin: if lag.is_some() {
            ORIGIN_CONFIG.to_string()
        } else {
            ORIGIN_NOT_STATED.to_string()
        },
        newest_date: newest,
        days_old,
        age_text,
        measurement_origin: ORIGIN_MEASURED.to_string(),
    })
}

fn source_county_fips(source_id: &str) -> Option<&'static str> {
    match source_id {
        "src-waukesha-parcels" => Some("55133"),
        "src-milwaukee-parcels" => Some("55079"),
        _ => None,
    }
}

fn three_value_nulls(conn: &Connection, county_fips: &str) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM current_parcels
         WHERE county_fips = ?1
           AND assessed_value IS NULL
           AND improvement_value IS NULL
           AND land_value IS NULL",
        [county_fips],
        |row| row.get(0),
    )?)
}

fn value_null_munis(conn: &Connection, county_fips: &str) -> Result<Vec<MuniMeasured>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT p.muni_code,
                COALESCE(m.muni_name, p.muni_code),
                COUNT(*)
         FROM current_parcels p
         LEFT JOIN muni_lookup m
            ON m.county_fips = p.county_fips
           AND m.muni_code = p.muni_code
         WHERE p.county_fips = ?1
         GROUP BY p.muni_code
         HAVING SUM(CASE
                    WHEN p.assessed_value IS NULL
                     AND p.improvement_value IS NULL
                     AND p.land_value IS NULL THEN 1
                    ELSE 0 END) = COUNT(*)
         ORDER BY COALESCE(m.muni_name, p.muni_code), p.muni_code",
    )?;
    let rows = stmt.query_map([county_fips], |row| {
        Ok(MuniMeasured {
            muni_code: row.get(0)?,
            name: row.get(1)?,
            row_count: row.get(2)?,
            value_as_of: None,
            origin: ORIGIN_MEASURED.to_string(),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn count_where(conn: &Connection, sql: &str, bind: &str) -> Result<i64, StoreError> {
    Ok(conn.query_row(sql, [bind], |row| row.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rank::tests::{test_bands, test_config};
    use crate::series::add_days;
    use radar_config::coverage::{CoverageEntry, CoverageLevel, PublicationLag};
    use radar_config::load::load;
    use radar_config::series::SeriesEntry;
    use rusqlite::params;
    use std::path::PathBuf;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        crate::migrate::migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (1, 'src-parcels', 'success', '2026-09-05')",
            [],
        )
        .expect("run");
        conn
    }

    fn insert_parcel(
        conn: &Connection,
        id: i64,
        county: &str,
        muni: &str,
        taxkey: &str,
        owner: Option<&str>,
        assessed: Option<f64>,
        improvement: Option<f64>,
        land: Option<f64>,
        prop_class: Option<&str>,
        value_source: Option<&str>,
        value_as_of: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO current_parcels (
                 county_fips, muni_code, taxkey, id, load_run_id,
                 owner_name, assessed_value, improvement_value, land_value,
                 prop_class, value_source, value_as_of, materialised_at
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?9, ?10, ?11, '2026-09-05')",
            params![
                county,
                muni,
                taxkey,
                id,
                owner,
                assessed,
                improvement,
                land,
                prop_class,
                value_source,
                value_as_of
            ],
        )
        .expect("parcel");
    }

    fn insert_muni(conn: &Connection, county: &str, code: &str, name: &str) {
        conn.execute(
            "INSERT INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
             VALUES (?1, ?2, ?3, 'test', 1)",
            params![county, code, name],
        )
        .expect("muni");
    }

    fn insert_obs(conn: &Connection, series_id: &str, date: &str) {
        conn.execute(
            "INSERT INTO series_observations (
                 series_id, observation_date, value, realtime_start, realtime_end,
                 load_run_id, fetched_at
             ) VALUES (?1, ?2, 1.0, '2026-09-05', '9999-12-31', 1, '2026-09-05T00:00:00+00:00')",
            params![series_id, date],
        )
        .expect("obs");
    }

    fn attr(key: &str, level: CoverageLevel, note: &str) -> CoverageEntry {
        CoverageEntry {
            key: key.to_string(),
            level,
            note: note.to_string(),
        }
    }

    fn geo(key: &str, level: CoverageLevel, note: &str) -> CoverageEntry {
        attr(key, level, note)
    }

    #[test]
    fn milwaukee_parcels_shows_milwaukee_coverage_not_waukesha() {
        let conn = open();
        insert_parcel(
            &conn, 1, "55079", "53000", "m1", Some("M"), None, None, None, Some("1"),
            Some("live"), None,
        );
        insert_parcel(
            &conn, 2, "55133", "WAKC", "w1", Some("W"), Some(10.0), Some(4.0), Some(6.0),
            Some("1"), Some("live"), Some("2026-08-10"),
        );
        let milwaukee_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/milwaukee_parcels");
        let waukesha_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/dr_data_icp");
        let milwaukee = load(&milwaukee_dir).expect("milwaukee config");
        let waukesha = load(&waukesha_dir).expect("waukesha config");
        let m_page = list_coverage(&milwaukee, &conn).expect("milwaukee page");
        let w_page = list_coverage(&waukesha, &conn).expect("waukesha page");
        let m_mil = m_page
            .geographies
            .iter()
            .find(|row| row.key == "milwaukee_county")
            .expect("milwaukee geography");
        let m_wauk = m_page
            .geographies
            .iter()
            .find(|row| row.key == "waukesha_county")
            .expect("waukesha geography on milwaukee page");
        assert_eq!(m_mil.level.as_deref(), Some("full"));
        assert!(m_mil.note.as_deref().unwrap().contains("No assessed"));
        assert_eq!(m_wauk.level.as_deref(), Some("none"));
        assert!(m_wauk.note.as_deref().unwrap().contains("This config is Milwaukee"));
        assert!(!m_mil.note.as_deref().unwrap().contains("Parcel and municipality layers loaded"));
        let w_wauk = w_page
            .geographies
            .iter()
            .find(|row| row.key == "waukesha_county")
            .expect("waukesha geography");
        assert_eq!(w_wauk.level.as_deref(), Some("full"));
        assert!(w_wauk
            .note
            .as_deref()
            .unwrap()
            .contains("Parcel and municipality layers loaded"));
        assert_eq!(m_page.county_fips, "55079");
        assert_eq!(w_page.county_fips, "55133");
        assert!(!m_page.geographies.iter().any(|row| row.key == "milc"));
        assert!(w_page.geographies.iter().any(|row| row.key == "milc"));
        assert_eq!(m_page.framing, GAP_FRAMING);
    }

    #[test]
    fn measured_null_count_that_disagrees_with_stated_level_is_shown_not_reconciled() {
        let conn = open();
        insert_parcel(
            &conn, 1, "55133", "WAKC", "a", Some("A"), Some(1.0), Some(1.0), Some(1.0),
            Some("1"), Some("live"), Some("2026-08-10"),
        );
        insert_parcel(
            &conn, 2, "55133", "WAKC", "b", Some("B"), None, None, None, Some("1"),
            Some("live"), Some("2026-08-10"),
        );
        insert_parcel(
            &conn, 3, "55133", "WAKC", "c", Some("C"), None, None, None, Some("1"),
            Some("live"), Some("2026-08-10"),
        );
        let mut config = test_config(1, "caveat", test_bands());
        config.coverage.geographies = vec![geo(
            "waukesha_county",
            CoverageLevel::Full,
            "stated full",
        )];
        config.coverage.attributes = vec![attr(
            "improvement_value",
            CoverageLevel::Full,
            "complete on all rows",
        )];
        config.coverage.publication_lags = vec![];
        let page = list_coverage(&config, &conn).expect("page");
        let improvement = page
            .attributes
            .iter()
            .find(|row| row.key == "improvement_value")
            .expect("improvement");
        assert_eq!(improvement.stated_level.as_deref(), Some("full"));
        assert_eq!(improvement.level_origin, ORIGIN_CONFIG);
        assert_eq!(improvement.measured_null_count, Some(2));
        assert_eq!(improvement.measured_row_count, Some(3));
        assert_eq!(improvement.measurement_origin, ORIGIN_MEASURED);
        assert!(improvement.disagreement);
        let text = improvement.disagreement_text.as_deref().expect("text");
        assert!(text.contains("stated full"));
        assert!(text.contains("2 of 3"));
        assert_eq!(improvement.stated_level.as_deref(), Some("full"));
    }

    #[test]
    fn store_attribute_absent_from_coverage_toml_is_listed_as_not_stated() {
        let conn = open();
        insert_parcel(
            &conn, 1, "55133", "WAKC", "a", Some("A"), Some(10.0), Some(4.0), Some(6.0),
            Some("1"), Some("live"), Some("2026-08-10"),
        );
        let mut config = test_config(1, "caveat", test_bands());
        config.coverage.geographies = vec![geo("waukesha_county", CoverageLevel::Full, "note")];
        config.coverage.attributes = vec![attr("owner_name", CoverageLevel::Full, "names")];
        config.coverage.publication_lags = vec![];
        let page = list_coverage(&config, &conn).expect("page");
        let keys: Vec<&str> = page.attributes.iter().map(|row| row.key.as_str()).collect();
        assert!(keys.contains(&"owner_name"));
        assert!(keys.contains(&"land_value"));
        let land = page
            .attributes
            .iter()
            .find(|row| row.key == "land_value")
            .expect("land_value");
        assert!(land.stated_level.is_none());
        assert_eq!(land.level_origin, ORIGIN_NOT_STATED);
        assert_eq!(land.measured_null_count, Some(0));
        assert_eq!(land.measurement_origin, ORIGIN_MEASURED);
        let owner = page
            .attributes
            .iter()
            .find(|row| row.key == "owner_name")
            .expect("owner");
        assert_eq!(owner.stated_level.as_deref(), Some("full"));
        assert_eq!(owner.level_origin, ORIGIN_CONFIG);
    }

    #[test]
    fn historic_municipalities_are_named_from_the_store() {
        let conn = open();
        insert_muni(&conn, "55133", "BRC", "Brookfield");
        insert_muni(&conn, "55133", "MILC", "MILC");
        insert_parcel(
            &conn, 1, "55133", "BRC", "b1", Some("B"), Some(1.0), Some(1.0), Some(1.0),
            Some("1"), Some("historic_year_end"), Some("2026-01-30"),
        );
        insert_parcel(
            &conn, 2, "55133", "BRC", "b2", Some("B"), Some(1.0), Some(1.0), Some(1.0),
            Some("1"), Some("historic_year_end"), Some("2026-01-30"),
        );
        insert_parcel(
            &conn, 3, "55133", "MILC", "m1", Some("M"), Some(1.0), Some(1.0), Some(1.0),
            None, Some("live"), Some("2026-08-10"),
        );
        insert_parcel(
            &conn, 4, "55133", "MILC", "m2", Some("M"), Some(1.0), Some(1.0), Some(1.0),
            None, Some("live"), Some("2026-08-10"),
        );
        insert_parcel(
            &conn, 5, "55133", "MILC", "m3", Some("M"), Some(1.0), Some(1.0), Some(1.0),
            None, Some("live"), Some("2026-08-10"),
        );
        let mut config = test_config(1, "caveat", test_bands());
        config.entity.county_fips = "55133".to_string();
        config.coverage.geographies = vec![
            geo("waukesha_county", CoverageLevel::Full, "loaded"),
            geo("milc", CoverageLevel::None, "uncovered"),
        ];
        config.coverage.attributes = vec![];
        config.coverage.publication_lags = vec![];
        let page = list_coverage(&config, &conn).expect("page");
        let county = page
            .geographies
            .iter()
            .find(|row| row.key == "waukesha_county")
            .expect("county");
        assert_eq!(county.historic_municipalities.len(), 1);
        assert_eq!(county.historic_municipalities[0].name, "Brookfield");
        assert_eq!(county.historic_municipalities[0].row_count, 2);
        assert_eq!(
            county.historic_municipalities[0].value_as_of.as_deref(),
            Some("2026-01-30")
        );
        assert_eq!(county.historic_municipalities[0].origin, ORIGIN_MEASURED);
        let milc = page
            .geographies
            .iter()
            .find(|row| row.key == "milc")
            .expect("milc");
        assert_eq!(milc.measured_row_count, 3);
        assert_eq!(milc.measured_classed_count, Some(0));
        assert_eq!(milc.level.as_deref(), Some("none"));
    }

    #[test]
    fn milwaukee_value_gaps_are_measured() {
        let conn = open();
        insert_muni(&conn, "55079", "OAK", "Oak Creek");
        insert_muni(&conn, "55079", "MKE", "Milwaukee");
        insert_parcel(
            &conn, 1, "55079", "OAK", "o1", Some("O"), None, None, None, Some("1"),
            Some("live"), None,
        );
        insert_parcel(
            &conn, 2, "55079", "MKE", "k1", Some("K"), None, None, None, Some("1"),
            Some("live"), None,
        );
        insert_parcel(
            &conn, 3, "55079", "MKE", "k2", Some("K"), Some(5.0), Some(3.0), Some(2.0),
            Some("1"), Some("live"), None,
        );
        let mut config = test_config(1, "caveat", test_bands());
        config.entity.county_fips = "55079".to_string();
        config.manifest.county_fips = "55079".to_string();
        config.manifest.county_name = "Milwaukee".to_string();
        config.coverage.geographies = vec![geo(
            "milwaukee_county",
            CoverageLevel::Full,
            "No Export_Date",
        )];
        config.coverage.attributes = vec![attr(
            "improvement_value",
            CoverageLevel::Partial,
            "Absent from the public primary view",
        )];
        config.coverage.publication_lags = vec![PublicationLag {
            source_id: "src-milwaukee-parcels".to_string(),
            lag_days: 0,
            measured: false,
            measured_from: None,
        }];
        let page = list_coverage(&config, &conn).expect("page");
        assert_eq!(page.three_value_null_count, 2);
        assert_eq!(page.three_value_null_origin, ORIGIN_MEASURED);
        let names: Vec<&str> = page
            .value_null_municipalities
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(names, vec!["Oak Creek"]);
        let value_as_of = page
            .attributes
            .iter()
            .find(|row| row.key == "value_as_of")
            .expect("value_as_of");
        assert_eq!(value_as_of.level_origin, ORIGIN_NOT_STATED);
        assert_eq!(value_as_of.measured_null_count, Some(3));
        let lag = page
            .publication_lags
            .iter()
            .find(|row| row.source_id == "src-milwaukee-parcels")
            .expect("lag");
        assert!(lag.age_text.contains("age cannot be computed"));
        assert_eq!(lag.stated_measured, Some(false));
        assert_eq!(lag.stated_origin, ORIGIN_CONFIG);
        assert_eq!(lag.measurement_origin, ORIGIN_MEASURED);
    }

    #[test]
    fn county_fred_series_age_is_measured() {
        let conn = open();
        let date = add_days(&today_ymd(), -650).expect("date");
        insert_obs(&conn, "BPPRIV055133", &date);
        let mut config = test_config(1, "caveat", test_bands());
        config.coverage.geographies = vec![];
        config.coverage.attributes = vec![];
        config.coverage.publication_lags = vec![];
        config.series.series = vec![SeriesEntry {
            series_id: "BPPRIV055133".to_string(),
            label: "Private housing units authorised".to_string(),
            geography: "county".to_string(),
            included: true,
            notes: None,
        }];
        let page = list_coverage(&config, &conn).expect("page");
        let lag = page
            .publication_lags
            .iter()
            .find(|row| row.series_id.as_deref() == Some("BPPRIV055133"))
            .expect("county series");
        assert_eq!(lag.days_old, Some(650));
        assert!(lag.age_text.contains("650"));
        assert_eq!(lag.measurement_origin, ORIGIN_MEASURED);
        assert_eq!(lag.stated_origin, ORIGIN_NOT_STATED);
    }
}
