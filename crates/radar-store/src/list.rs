use std::fs;
use std::path::Path;

use radar_config::{Direction, Favourability, RadarConfig, Zone};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::evaluate::{evaluate_attribute, AttributeEvaluation, EvaluateError};
use crate::groups::{format_value_as_of_range, OwnerGroup};
use crate::normalise::owner_group_key;
use crate::outcomes::{latest_outcomes, LatestOutcome, SUBJECT_OWNER_GROUP};
use crate::rank::favourability_order;
use crate::StoreError;

pub const RANKING_CAPABILITY_KEY: &str = "prospect_ranking_by_likelihood_to_buy";
pub const DEFAULT_PAGE_LIMIT: i64 = 100;

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Evaluate(#[from] EvaluateError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum BandFilter {
    #[serde(rename = "band")]
    Band { label: String },
    #[serde(rename = "out_of_bands")]
    OutOfBands,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum BandState {
    #[serde(rename = "band")]
    Band {
        label: String,
        favourability: String,
        value: f64,
    },
    #[serde(rename = "out_of_bands")]
    OutOfBands { value: f64 },
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupListQuery {
    pub county_fips: Option<String>,
    pub municipality: Option<String>,
    pub owner_type: Option<String>,
    pub band: Option<BandFilter>,
    pub min_parcels: Option<i64>,
    pub min_improvement_value_zero: Option<i64>,
    pub name_contains: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    #[serde(default)]
    pub apply_default_owner_type_exclusions: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupListRow {
    pub group_key: String,
    pub display_name: String,
    pub member_name_count: i64,
    pub total_parcels: Option<i64>,
    pub improvement_value_zero_count: Option<i64>,
    pub municipality_count: i64,
    pub mixed_owner_type: bool,
    pub owner_type: Option<String>,
    pub band: BandState,
    pub value_as_of: Option<String>,
    pub latest_outcome: Option<LatestOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankingCaveat {
    pub assignment_found: bool,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupListPage {
    pub rows: Vec<GroupListRow>,
    pub total_before_filter: i64,
    pub total_after_filter: i64,
    pub offset: i64,
    pub limit: i64,
    pub predicate_chain: String,
    pub caveat: RankingCaveat,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MunicipalityOption {
    pub muni_code: String,
    pub muni_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CountyOption {
    pub county_fips: String,
    pub county_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupFilterOptions {
    pub counties: Vec<CountyOption>,
    pub municipalities: Vec<MunicipalityOption>,
    pub owner_types: Vec<String>,
    pub bands: Vec<BandFilter>,
    pub caveat: RankingCaveat,
    pub county_fips: String,
    pub county_name: String,
    pub vertical_display_name: String,
    pub config_version: String,
    pub default_excluded_owner_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupParcelRow {
    pub taxkey: Option<String>,
    pub site_address: Option<String>,
    pub municipality: Option<String>,
    pub owner_name: Option<String>,
    pub assessed_value: Option<f64>,
    pub improvement_value: Option<f64>,
    pub land_value: Option<f64>,
    pub prop_class: Option<String>,
    pub value_source: Option<String>,
    pub value_as_of: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupExportResult {
    pub csv_path: String,
    pub provenance_path: String,
    pub rows_exported: i64,
    pub predicate_chain: String,
}

pub fn ranking_caveat(config: &RadarConfig) -> RankingCaveat {
    let assignment = config.zones.assignments.iter().find(|assignment| {
        assignment.capability_key == RANKING_CAPABILITY_KEY && assignment.zone == Zone::Below
    });
    match assignment {
        Some(found) => RankingCaveat {
            assignment_found: true,
            text: found.display_caveat.clone(),
        },
        None => RankingCaveat {
            assignment_found: false,
            text: None,
        },
    }
}

fn alloc_placeholder(next: &mut i32) -> String {
    let index = *next;
    *next += 1;
    format!("?{index}")
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

struct BandSql {
    kind: String,
    label: String,
    favourability: String,
    value: String,
    order: String,
}

fn range_sql(column: &str, min: Option<f64>, max: Option<f64>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{column} >= {min} AND {column} <= {max}"),
        (Some(min), None) => format!("{column} >= {min}"),
        (None, Some(max)) => format!("{column} <= {max}"),
        (None, None) => "1 = 1".to_string(),
    }
}

fn band_sql(config: &RadarConfig, zero_col: &str) -> BandSql {
    let Some(attribute) = band_attribute(config) else {
        return BandSql {
            kind: "'unknown'".to_string(),
            label: "NULL".to_string(),
            favourability: "NULL".to_string(),
            value: "NULL".to_string(),
            order: "3".to_string(),
        };
    };
    let mut kind = format!("CASE WHEN {zero_col} IS NULL THEN 'unknown'");
    let mut label = format!("CASE WHEN {zero_col} IS NULL THEN NULL");
    let mut favourability = format!("CASE WHEN {zero_col} IS NULL THEN NULL");
    let mut value = format!("CASE WHEN {zero_col} IS NULL THEN NULL");
    let mut order = format!("CASE WHEN {zero_col} IS NULL THEN 3");
    if let Some(bands) = attribute.bands.as_ref() {
        for band in bands {
            let cond = range_sql(zero_col, band.min, band.max);
            let fav = favourability_label(&band.favourability);
            let fav_order = favourability_order(Some(&band.favourability));
            kind.push_str(&format!(" WHEN {cond} THEN 'band'"));
            label.push_str(&format!(" WHEN {cond} THEN {}", sql_string(&band.label)));
            favourability.push_str(&format!(" WHEN {cond} THEN {}", sql_string(&fav)));
            value.push_str(&format!(" WHEN {cond} THEN CAST({zero_col} AS REAL)"));
            order.push_str(&format!(" WHEN {cond} THEN {fav_order}"));
        }
    }
    kind.push_str(" ELSE 'out_of_bands' END");
    label.push_str(" ELSE NULL END");
    favourability.push_str(" ELSE NULL END");
    value.push_str(&format!(" ELSE CAST({zero_col} AS REAL) END"));
    order.push_str(" ELSE 3 END");
    BandSql {
        kind,
        label,
        favourability,
        value,
        order,
    }
}

fn band_from_sql(
    kind: &str,
    label: Option<String>,
    favourability: Option<String>,
    value: Option<f64>,
) -> BandState {
    match kind {
        "band" => BandState::Band {
            label: label.unwrap_or_default(),
            favourability: favourability.unwrap_or_default(),
            value: value.unwrap_or(0.0),
        },
        "out_of_bands" => BandState::OutOfBands {
            value: value.unwrap_or(0.0),
        },
        _ => BandState::Unknown,
    }
}

fn row_i64(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<i64> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Null => Ok(0),
        rusqlite::types::ValueRef::Integer(value) => Ok(value),
        rusqlite::types::ValueRef::Real(value) => Ok(value as i64),
        other => Err(rusqlite::Error::InvalidColumnType(
            idx,
            other.data_type().to_string(),
            rusqlite::types::Type::Integer,
        )),
    }
}

fn row_opt_i64(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<i64>> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Null => Ok(None),
        rusqlite::types::ValueRef::Integer(value) => Ok(Some(value)),
        rusqlite::types::ValueRef::Real(value) => Ok(Some(value as i64)),
        other => Err(rusqlite::Error::InvalidColumnType(
            idx,
            other.data_type().to_string(),
            rusqlite::types::Type::Integer,
        )),
    }
}

fn register_owner_group_sql_functions(
    conn: &Connection,
    owner_type: &radar_config::OwnerTypeFile,
) -> Result<(), rusqlite::Error> {
    use rusqlite::functions::FunctionFlags;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    conn.create_scalar_function("normalise_owner_name", 1, flags, |ctx| {
        let raw: Option<String> = ctx.get(0)?;
        Ok(raw
            .map(|value| owner_group_key(&value))
            .unwrap_or_default())
    })?;
    let types = owner_type.compiled();
    conn.create_scalar_function("classify_owner_type", 1, flags, move |ctx| {
        let raw: Option<String> = ctx.get(0)?;
        Ok(raw
            .map(|value| types.classify(&value))
            .unwrap_or_else(|| "individual".to_string()))
    })?;
    Ok(())
}

pub fn list_filter_options(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<GroupFilterOptions, ListError> {
    let county_fips = config.manifest.county_fips.as_str();
    let mut muni_stmt = conn.prepare(
        "SELECT muni_code, muni_name
         FROM muni_lookup
         WHERE county_fips = ?1
         ORDER BY muni_name, muni_code",
    )?;
    let muni_rows = muni_stmt.query_map([county_fips], |row| {
        Ok(MunicipalityOption {
            muni_code: row.get(0)?,
            muni_name: row.get(1)?,
        })
    })?;
    let mut municipalities = Vec::new();
    for row in muni_rows {
        municipalities.push(row?);
    }

    let mut owner_types: Vec<String> = config
        .owner_type
        .classes
        .iter()
        .map(|class| class.type_key.clone())
        .collect();
    owner_types.push("individual".to_string());
    owner_types.sort();
    owner_types.dedup();

    let mut bands = Vec::new();
    if let Some(attribute) = band_attribute(config) {
        if let Some(declared) = attribute.bands.as_ref() {
            for band in declared {
                bands.push(BandFilter::Band {
                    label: band.label.clone(),
                });
            }
        }
    }
    bands.push(BandFilter::OutOfBands);
    bands.push(BandFilter::Unknown);

    Ok(GroupFilterOptions {
        counties: vec![CountyOption {
            county_fips: county_fips.to_string(),
            county_name: config.manifest.county_name.clone(),
        }],
        municipalities,
        owner_types,
        bands,
        caveat: ranking_caveat(config),
        county_fips: county_fips.to_string(),
        county_name: config.manifest.county_name.clone(),
        vertical_display_name: config.manifest.vertical_display_name.clone(),
        config_version: config.manifest.config_version.clone(),
        default_excluded_owner_types: config.owner_type.exclude_from_groups_default.clone(),
    })
}

pub fn list_owner_groups(
    config: &RadarConfig,
    conn: &Connection,
    query: &GroupListQuery,
) -> Result<GroupListPage, ListError> {
    register_owner_group_sql_functions(conn, &config.owner_type)?;
    let county_fips = config.manifest.county_fips.as_str();
    let cleaned = clean_query(query);
    let excluded = if cleaned.apply_default_owner_type_exclusions.unwrap_or(true)
        && cleaned.owner_type.is_none()
    {
        config.owner_type.exclude_from_groups_default.as_slice()
    } else {
        &[]
    };
    let offset = cleaned.offset.unwrap_or(0).max(0);
    let limit = match cleaned.limit {
        Some(value) if value > 0 => value,
        _ => DEFAULT_PAGE_LIMIT,
    };
    let band = band_sql(config, "scored.improvement_value_zero_count");

    let mut param_i = 1;
    let county_ph = alloc_placeholder(&mut param_i);
    let mut binds: Vec<rusqlite::types::Value> =
        vec![rusqlite::types::Value::Text(county_fips.to_string())];

    let mut filters = Vec::new();
    if let Some(muni) = cleaned.municipality.as_deref() {
        let ph = alloc_placeholder(&mut param_i);
        filters.push(format!(
            "EXISTS (
                 SELECT 1 FROM current_parcels c
                 JOIN mapped m ON m.owner_name = c.owner_name
                 WHERE m.group_key = g.group_key
                   AND c.muni_code = {ph}
                   AND c.county_fips = {county_ph}
             )"
        ));
        binds.push(rusqlite::types::Value::Text(muni.to_string()));
    }
    if let Some(owner_type) = cleaned.owner_type.as_deref() {
        let ph = alloc_placeholder(&mut param_i);
        filters.push(format!("g.owner_type = {ph}"));
        binds.push(rusqlite::types::Value::Text(owner_type.to_string()));
    } else if !excluded.is_empty() {
        let mut placeholders = Vec::new();
        for item in excluded {
            placeholders.push(alloc_placeholder(&mut param_i));
            binds.push(rusqlite::types::Value::Text(item.clone()));
        }
        filters.push(format!(
            "(g.owner_type IS NULL OR g.owner_type NOT IN ({}))",
            placeholders.join(", ")
        ));
    }
    if let Some(min) = cleaned.min_parcels {
        let ph = alloc_placeholder(&mut param_i);
        filters.push(format!("g.total_parcels >= {ph}"));
        binds.push(rusqlite::types::Value::Integer(min));
    }
    if let Some(min) = cleaned.min_improvement_value_zero {
        let ph = alloc_placeholder(&mut param_i);
        filters.push(format!("g.improvement_value_zero_count >= {ph}"));
        binds.push(rusqlite::types::Value::Integer(min));
    }
    if let Some(needle) = cleaned.name_contains.as_deref() {
        let normalised = owner_group_key(needle);
        let ph = alloc_placeholder(&mut param_i);
        filters.push(format!("instr(g.group_key, {ph}) > 0"));
        binds.push(rusqlite::types::Value::Text(normalised));
    }
    match cleaned.band.as_ref() {
        Some(BandFilter::Band { label }) => {
            let ph = alloc_placeholder(&mut param_i);
            filters.push(format!("g.band_kind = 'band' AND g.band_label = {ph}"));
            binds.push(rusqlite::types::Value::Text(label.clone()));
        }
        Some(BandFilter::OutOfBands) => filters.push("g.band_kind = 'out_of_bands'".to_string()),
        Some(BandFilter::Unknown) => filters.push("g.band_kind = 'unknown'".to_string()),
        None => {}
    }
    let where_filtered = if filters.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", filters.join(" AND "))
    };
    let limit_ph = alloc_placeholder(&mut param_i);
    let offset_ph = alloc_placeholder(&mut param_i);
    binds.push(rusqlite::types::Value::Integer(limit));
    binds.push(rusqlite::types::Value::Integer(offset));

    let sql = format!(
        "WITH names AS MATERIALIZED (
            SELECT
                owner_name,
                COUNT(*) AS n,
                SUM(CASE WHEN improvement_value IS NOT NULL THEN 1 ELSE 0 END) AS improvement_observed,
                SUM(CASE WHEN improvement_value = 0 THEN 1 ELSE 0 END) AS zero_count,
                MIN(value_as_of) AS value_as_of_min,
                MAX(value_as_of) AS value_as_of_max
            FROM current_parcels
            WHERE county_fips = {county_ph}
              AND owner_name IS NOT NULL
            GROUP BY owner_name
        ),
        mapped AS MATERIALIZED (
            SELECT
                owner_name,
                n,
                improvement_observed,
                zero_count,
                value_as_of_min,
                value_as_of_max,
                normalise_owner_name(owner_name) AS group_key,
                classify_owner_type(owner_name) AS owner_type
            FROM names
            WHERE normalise_owner_name(owner_name) != ''
        ),
        agg AS MATERIALIZED (
            SELECT
                group_key,
                SUM(n) AS total_parcels,
                SUM(improvement_observed) AS improvement_observed,
                SUM(zero_count) AS zero_count,
                COUNT(*) AS member_name_count,
                COUNT(DISTINCT owner_type) AS owner_type_n,
                MIN(owner_type) AS owner_type_min,
                MIN(value_as_of_min) AS value_as_of_min,
                MAX(value_as_of_max) AS value_as_of_max
            FROM mapped
            GROUP BY group_key
        ),
        grouped AS (
            SELECT
                scored.group_key,
                scored.member_name_count,
                scored.total_parcels,
                scored.improvement_value_zero_count,
                scored.mixed_owner_type,
                scored.owner_type,
                scored.value_as_of_min,
                scored.value_as_of_max,
                {kind} AS band_kind,
                {label} AS band_label,
                {favourability} AS band_favourability,
                {value} AS band_value,
                {order} AS fav_order
            FROM (
                SELECT
                    a.group_key,
                    a.member_name_count,
                    a.total_parcels,
                    CASE WHEN a.improvement_observed = 0 THEN NULL ELSE a.zero_count END
                        AS improvement_value_zero_count,
                    CASE WHEN a.owner_type_n > 1 THEN 1 ELSE 0 END AS mixed_owner_type,
                    CASE WHEN a.owner_type_n > 1 THEN NULL ELSE a.owner_type_min END AS owner_type,
                    a.value_as_of_min,
                    a.value_as_of_max
                FROM agg a
            ) scored
        ),
        page AS MATERIALIZED (
            SELECT *
            FROM grouped g
            {where_filtered}
            ORDER BY fav_order ASC, improvement_value_zero_count DESC, total_parcels DESC, group_key ASC
            LIMIT {limit_ph} OFFSET {offset_ph}
        ),
        display AS (
            SELECT group_key, owner_name AS display_name
            FROM (
                SELECT
                    m.group_key,
                    m.owner_name,
                    ROW_NUMBER() OVER (
                        PARTITION BY m.group_key
                        ORDER BY m.n DESC, m.owner_name ASC
                    ) AS rn
                FROM mapped m
                INNER JOIN page p ON p.group_key = m.group_key
            ) ranked
            WHERE rn = 1
        ),
        munis AS (
            SELECT m.group_key, COUNT(DISTINCT c.muni_code) AS municipality_count
            FROM current_parcels c
            JOIN mapped m ON m.owner_name = c.owner_name
            JOIN page p ON p.group_key = m.group_key
            WHERE c.county_fips = {county_ph}
            GROUP BY m.group_key
        )
        SELECT
            (SELECT COUNT(*) FROM grouped) AS total_before_filter,
            (SELECT COUNT(*) FROM grouped g {where_filtered}) AS total_after_filter,
            page.group_key,
            display.display_name,
            page.member_name_count,
            page.total_parcels,
            page.improvement_value_zero_count,
            munis.municipality_count,
            page.mixed_owner_type,
            page.owner_type,
            page.value_as_of_min,
            page.value_as_of_max,
            page.band_kind,
            page.band_label,
            page.band_favourability,
            page.band_value
        FROM (SELECT 1 AS _pad) AS pad
        LEFT JOIN page ON 1 = 1
        LEFT JOIN display ON display.group_key = page.group_key
        LEFT JOIN munis ON munis.group_key = page.group_key",
        kind = band.kind,
        label = band.label,
        favourability = band.favourability,
        value = band.value,
        order = band.order,
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(binds.iter()))?;
    let mut page_rows = Vec::new();
    let mut total_before_filter = 0i64;
    let mut total_after_filter = 0i64;
    while let Some(row) = rows.next()? {
        total_before_filter = row_i64(row, 0)?;
        total_after_filter = row_i64(row, 1)?;
        let group_key: Option<String> = row.get(2)?;
        let Some(group_key) = group_key else {
            continue;
        };
        let display_name: String = row.get(3)?;
        let member_name_count = row_i64(row, 4)?;
        let total_parcels = row_i64(row, 5)?;
        let improvement_value_zero_count = row_opt_i64(row, 6)?;
        let municipality_count = row_i64(row, 7)?;
        let mixed_owner_type = row_i64(row, 8)?;
        let owner_type: Option<String> = row.get(9)?;
        let value_as_of_min: Option<String> = row.get(10)?;
        let value_as_of_max: Option<String> = row.get(11)?;
        let band_kind: String = row.get(12)?;
        let band_label: Option<String> = row.get(13)?;
        let band_favourability: Option<String> = row.get(14)?;
        let band_value: Option<f64> = row.get(15)?;
        page_rows.push(GroupListRow {
            group_key,
            display_name,
            member_name_count,
            total_parcels: Some(total_parcels),
            improvement_value_zero_count,
            municipality_count,
            mixed_owner_type: mixed_owner_type != 0,
            owner_type,
            band: band_from_sql(&band_kind, band_label, band_favourability, band_value),
            value_as_of: format_value_as_of_range(
                value_as_of_min.as_deref(),
                value_as_of_max.as_deref(),
            ),
            latest_outcome: None,
        });
    }
    let keys: Vec<String> = page_rows.iter().map(|row| row.group_key.clone()).collect();
    let latest = latest_outcomes(conn, SUBJECT_OWNER_GROUP, &keys)?;
    for row in &mut page_rows {
        row.latest_outcome = latest.get(&row.group_key).cloned();
    }

    Ok(GroupListPage {
        rows: page_rows,
        total_before_filter,
        total_after_filter,
        offset,
        limit,
        predicate_chain: predicate_chain(county_fips, &cleaned, offset, limit, excluded),
        caveat: ranking_caveat(config),
    })
}

fn explain_sql_plan(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
    let rows = stmt.query_map(params, |row| row.get::<_, String>(3))?;
    let mut plan = Vec::new();
    for row in rows {
        plan.push(row?);
    }
    Ok(plan)
}

pub fn explain_owner_group_list_plan(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<Vec<String>, ListError> {
    register_owner_group_sql_functions(conn, &config.owner_type)?;
    let county_fips = config.manifest.county_fips.as_str();
    let mut plan = Vec::new();
    for line in explain_sql_plan(
        conn,
        "SELECT owner_name, muni_code, improvement_value, value_as_of
         FROM current_parcels
         WHERE county_fips = ?1
           AND owner_name IS NOT NULL",
        &[&county_fips],
    )? {
        plan.push(format!("county_scan {line}"));
    }
    for line in explain_sql_plan(
        conn,
        "SELECT owner_name,
                COUNT(*) AS n,
                SUM(CASE WHEN improvement_value IS NOT NULL THEN 1 ELSE 0 END),
                SUM(CASE WHEN improvement_value = 0 THEN 1 ELSE 0 END),
                MIN(value_as_of),
                MAX(value_as_of)
         FROM current_parcels
         WHERE county_fips = ?1
           AND owner_name IS NOT NULL
         GROUP BY owner_name",
        &[&county_fips],
    )? {
        plan.push(format!("name_group {line}"));
    }
    for line in explain_sql_plan(
        conn,
        "SELECT normalise_owner_name(owner_name) AS group_key, COUNT(*) AS n
         FROM current_parcels
         WHERE county_fips = ?1
           AND owner_name IS NOT NULL
           AND normalise_owner_name(owner_name) != ''
         GROUP BY group_key
         ORDER BY n DESC, group_key ASC
         LIMIT 100",
        &[&county_fips],
    )? {
        plan.push(format!("group_page {line}"));
    }
    Ok(plan)
}

pub fn list_group_parcels(
    config: &RadarConfig,
    conn: &Connection,
    group_key: &str,
) -> Result<Vec<GroupParcelRow>, ListError> {
    register_owner_group_sql_functions(conn, &config.owner_type)?;
    let county_fips = config.manifest.county_fips.as_str();
    let mut stmt = conn.prepare(
        "SELECT e.taxkey, e.site_address, m.muni_name, e.owner_name,
                e.assessed_value, e.improvement_value, e.land_value,
                e.prop_class, e.value_source, e.value_as_of
         FROM current_parcels e
         LEFT JOIN muni_lookup m
           ON m.muni_code = e.muni_code AND m.county_fips = e.county_fips
         WHERE e.county_fips = ?1
           AND e.owner_name IS NOT NULL
           AND normalise_owner_name(e.owner_name) = ?2
         ORDER BY e.taxkey",
    )?;
    let rows = stmt.query_map(rusqlite::params![county_fips, group_key], |row| {
        Ok(GroupParcelRow {
            taxkey: row.get(0)?,
            site_address: row.get(1)?,
            municipality: row.get(2)?,
            owner_name: row.get(3)?,
            assessed_value: row.get(4)?,
            improvement_value: row.get(5)?,
            land_value: row.get(6)?,
            prop_class: row.get(7)?,
            value_source: row.get(8)?,
            value_as_of: row.get(9)?,
        })
    })?;
    let mut parcels = Vec::new();
    for row in rows {
        parcels.push(row?);
    }
    Ok(parcels)
}

pub fn export_owner_groups(
    config: &RadarConfig,
    conn: &Connection,
    query: &GroupListQuery,
    path: &Path,
) -> Result<GroupExportResult, ListError> {
    let mut export_query = query.clone();
    export_query.offset = Some(0);
    export_query.limit = Some(i64::MAX);
    let page = list_owner_groups(config, conn, &export_query)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut csv = String::new();
    csv.push_str(
        "group_key,display_name,member_name_count,total_parcels,improvement_value_zero_count,municipality_count,mixed_owner_type,owner_type,band_kind,band_label,band_favourability,band_value,value_as_of\n",
    );
    for row in &page.rows {
        let (kind, label, favourability, value) = band_csv_cells(&row.band);
        csv.push_str(&csv_row(&[
            row.group_key.clone(),
            row.display_name.clone(),
            row.member_name_count.to_string(),
            option_i64(row.total_parcels),
            option_i64(row.improvement_value_zero_count),
            row.municipality_count.to_string(),
            if row.mixed_owner_type {
                "true".to_string()
            } else {
                "false".to_string()
            },
            row.owner_type.clone().unwrap_or_default(),
            kind,
            label,
            favourability,
            value,
            row.value_as_of.clone().unwrap_or_default(),
        ]));
        csv.push('\n');
    }
    fs::write(path, csv)?;
    let provenance_path = provenance_path(path);
    let provenance = format!(
        "exported_at={}\nconfig_version={}\nrows_exported={}\npredicate_chain:\n{}\ndisplay_caveat={}\n",
        conn.query_row("SELECT datetime('now')", [], |row| row.get::<_, String>(0))
            .map_err(StoreError::from)?,
        config.manifest.config_version,
        page.rows.len(),
        page.predicate_chain,
        page.caveat.text.clone().unwrap_or_default(),
    );
    fs::write(&provenance_path, provenance)?;
    Ok(GroupExportResult {
        csv_path: path.display().to_string(),
        provenance_path: provenance_path.display().to_string(),
        rows_exported: page.rows.len() as i64,
        predicate_chain: page.predicate_chain,
    })
}

fn band_attribute(config: &RadarConfig) -> Option<&radar_config::Attribute> {
    config
        .attributes
        .attributes
        .iter()
        .find(|attribute| attribute.direction == Direction::Band)
}

#[allow(dead_code)]
fn resolve_band(config: &RadarConfig, group: &OwnerGroup) -> Result<BandState, ListError> {
    let Some(attribute) = band_attribute(config) else {
        return Ok(BandState::Unknown);
    };
    match evaluate_attribute(config, &attribute.key, group)? {
        AttributeEvaluation::Band {
            value,
            label,
            favourability,
        } => Ok(BandState::Band {
            label,
            favourability: favourability_label(&favourability),
            value,
        }),
        AttributeEvaluation::OutOfBands { value } => Ok(BandState::OutOfBands { value }),
        AttributeEvaluation::Unknown => Ok(BandState::Unknown),
        AttributeEvaluation::Value { value } => Ok(BandState::OutOfBands { value }),
    }
}

fn favourability_label(value: &Favourability) -> String {
    match value {
        Favourability::Favourable => "Favourable".to_string(),
        Favourability::Neutral => "Neutral".to_string(),
        Favourability::Unfavourable => "Unfavourable".to_string(),
    }
}

#[allow(dead_code)]
fn band_favourability(band: &BandState) -> Option<Favourability> {
    match band {
        BandState::Band { favourability, .. } => match favourability.as_str() {
            "Favourable" => Some(Favourability::Favourable),
            "Neutral" => Some(Favourability::Neutral),
            "Unfavourable" => Some(Favourability::Unfavourable),
            _ => None,
        },
        BandState::OutOfBands { .. } | BandState::Unknown => None,
    }
}

fn clean_query(query: &GroupListQuery) -> GroupListQuery {
    GroupListQuery {
        county_fips: nonempty(query.county_fips.clone()),
        municipality: nonempty(query.municipality.clone()),
        owner_type: nonempty(query.owner_type.clone()),
        band: query.band.clone(),
        min_parcels: query.min_parcels,
        min_improvement_value_zero: query.min_improvement_value_zero,
        name_contains: nonempty(query.name_contains.clone()),
        offset: query.offset,
        limit: query.limit,
        apply_default_owner_type_exclusions: query.apply_default_owner_type_exclusions,
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

#[allow(dead_code)]
fn matches_filters(
    group: &OwnerGroup,
    band: &BandState,
    loaded_county: &str,
    query: &GroupListQuery,
    excluded_owner_types: &[String],
) -> bool {
    if let Some(county) = query.county_fips.as_deref() {
        if county != loaded_county {
            return false;
        }
    }
    if let Some(muni) = query.municipality.as_deref() {
        if !group.muni_codes.iter().any(|code| code == muni) {
            return false;
        }
    }
    if let Some(owner_type) = query.owner_type.as_deref() {
        if group.owner_type.as_deref() != Some(owner_type) {
            return false;
        }
    } else {
        if let Some(group_type) = group.owner_type.as_deref() {
            if excluded_owner_types.iter().any(|excluded| excluded == group_type) {
                return false;
            }
        }
    }
    if let Some(wanted) = query.band.as_ref() {
        if !band_matches(band, wanted) {
            return false;
        }
    }
    if let Some(min) = query.min_parcels {
        match group.total_parcels {
            Some(count) if count >= min => {}
            _ => return false,
        }
    }
    if let Some(min) = query.min_improvement_value_zero {
        match group.improvement_value_zero_count {
            Some(count) if count >= min => {}
            _ => return false,
        }
    }
    if let Some(needle) = query.name_contains.as_deref() {
        let normalised = owner_group_key(needle);
        if normalised.is_empty() || !group.group_key.contains(&normalised) {
            return false;
        }
    }
    true
}

#[allow(dead_code)]
fn band_matches(band: &BandState, wanted: &BandFilter) -> bool {
    match (band, wanted) {
        (BandState::Band { label, .. }, BandFilter::Band { label: want }) => label == want,
        (BandState::OutOfBands { .. }, BandFilter::OutOfBands) => true,
        (BandState::Unknown, BandFilter::Unknown) => true,
        _ => false,
    }
}

fn predicate_chain(
    county_fips: &str,
    query: &GroupListQuery,
    offset: i64,
    limit: i64,
    excluded_owner_types: &[String],
) -> String {
    let mut lines = vec![
        format!(
            "SELECT owner_name, muni_code, improvement_value, value_as_of FROM current_parcels WHERE county_fips = {county_fips}"
        ),
        "group by normalise(owner_name)".to_string(),
        "owner_type classified at query time from owner_type.toml".to_string(),
    ];
    if let Some(county) = query.county_fips.as_deref() {
        lines.push(format!("county_fips = {county}"));
    }
    if let Some(muni) = query.municipality.as_deref() {
        lines.push(format!("group has muni_code = {muni}"));
    }
    if let Some(owner_type) = query.owner_type.as_deref() {
        lines.push(format!("owner_type = {owner_type}"));
    } else if !excluded_owner_types.is_empty() {
        lines.push(format!(
            "excluded owner types are hidden by default (exclude {})",
            excluded_owner_types.join(", ")
        ));
    }
    if let Some(band) = query.band.as_ref() {
        lines.push(format!("band_state = {}", band_filter_text(band)));
    }
    if let Some(min) = query.min_parcels {
        lines.push(format!("total_parcels >= {min}"));
    }
    if let Some(min) = query.min_improvement_value_zero {
        lines.push(format!("improvement_value_zero_count >= {min}"));
    }
    if let Some(name) = query.name_contains.as_deref() {
        lines.push(format!(
            "owner_group_key contains {}",
            owner_group_key(name)
        ));
    }
    lines.push(
        "order by band_favourability, improvement_value_zero_count desc, total_parcels desc, group_key"
            .to_string(),
    );
    let limit_text = if limit == i64::MAX {
        "all".to_string()
    } else {
        limit.to_string()
    };
    lines.push(format!("offset {offset} limit {limit_text}"));
    lines.join("\n")
}

fn band_filter_text(band: &BandFilter) -> String {
    match band {
        BandFilter::Band { label } => format!("band:{label}"),
        BandFilter::OutOfBands => "OutOfBands".to_string(),
        BandFilter::Unknown => "Unknown".to_string(),
    }
}

fn band_csv_cells(band: &BandState) -> (String, String, String, String) {
    match band {
        BandState::Band {
            label,
            favourability,
            value,
        } => (
            "band".to_string(),
            label.clone(),
            favourability.clone(),
            value.to_string(),
        ),
        BandState::OutOfBands { value } => (
            "out_of_bands".to_string(),
            String::new(),
            String::new(),
            value.to_string(),
        ),
        BandState::Unknown => (
            "unknown".to_string(),
            String::new(),
            String::new(),
            String::new(),
        ),
    }
}

fn option_i64(value: Option<i64>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => String::new(),
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

impl From<rusqlite::Error> for ListError {
    fn from(err: rusqlite::Error) -> Self {
        ListError::Store(StoreError::from(err))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::Instant;

    use radar_config::load;
    use radar_config::zones::{Zone, ZoneAssignment, Zones};
    use rusqlite::Connection;

    use super::{
        export_owner_groups, list_filter_options, list_group_parcels, list_owner_groups, BandFilter,
        BandState, GroupListQuery, DEFAULT_PAGE_LIMIT,
    };
    use crate::migrate::migrate;
    use crate::rank::tests::{test_bands, test_config};

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at, value_as_of)
             VALUES (1, 'src-test', 'success', '2026-08-10', '2026-08-10')",
            [],
        )
        .expect("run");
        conn.execute(
            "INSERT INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
             VALUES ('55133', 'WAKC', 'Waukesha City', 'test', 1)",
            [],
        )
        .expect("muni");
        conn.execute(
            "INSERT INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
             VALUES ('55133', 'NBC', 'New Berlin', 'test', 1)",
            [],
        )
        .expect("muni");
        conn
    }

    fn add_parcel(
        conn: &Connection,
        taxkey: &str,
        name: &str,
        owner_type: &str,
        muni: &str,
        improvement: f64,
        value_as_of: &str,
        county: &str,
    ) {
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value,
                 county_fips, value_as_of, site_address, assessed_value, land_value, prop_class,
                 value_source
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, '1 MAIN', 10, 4, '2', 'live')",
            rusqlite::params![
                taxkey,
                name,
                owner_type,
                muni,
                improvement,
                county,
                value_as_of
            ],
        )
        .expect("parcel");
        crate::current::mirror_latest_entity(conn).expect("mirror current_parcels");
    }

    #[test]
    fn page_default_limit_and_total_counts() {
        let conn = setup();
        for i in 0..12 {
            add_parcel(
                &conn,
                &format!("t{i}"),
                &format!("OWNER {i} LLC"),
                "entity",
                "WAKC",
                0.0,
                "2026-08-10",
                "55133",
            );
        }
        add_parcel(
            &conn,
            "other",
            "OTHER COUNTY LLC",
            "entity",
            "53000",
            0.0,
            "2026-08-10",
            "55079",
        );
        let config = test_config(1, "caveat from config", test_bands());
        let page = list_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                limit: Some(5),
                offset: Some(0),
                ..GroupListQuery::default()
            },
        )
        .expect("page");
        assert_eq!(page.total_before_filter, 12);
        assert_eq!(page.total_after_filter, 12);
        assert_eq!(page.rows.len(), 5);
        assert_eq!(page.limit, 5);
        assert!(page.predicate_chain.contains("county_fips = 55133"));
        assert!(page.caveat.assignment_found);
        assert_eq!(page.caveat.text.as_deref(), Some("caveat from config"));
    }

    #[test]
    fn default_filter_excludes_government_and_reports_counts() {
        let conn = setup();
        add_parcel(&conn, "e1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        add_parcel(
            &conn,
            "g1",
            "WAUKESHA COUNTY",
            "individual",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        let config = test_config(1, "caveat from config", test_bands());
        let page = list_owner_groups(&config, &conn, &GroupListQuery::default()).expect("page");
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 1);
        assert_eq!(page.rows[0].display_name, "ACME LLC");
        assert!(page
            .predicate_chain
            .contains("excluded owner types are hidden by default"));
        let options = list_filter_options(&config, &conn).expect("options");
        assert!(options
            .default_excluded_owner_types
            .contains(&"government".to_string()));
        assert!(options
            .default_excluded_owner_types
            .contains(&"placeholder".to_string()));
        assert!(options.owner_types.contains(&"government".to_string()));
        assert!(options.owner_types.contains(&"placeholder".to_string()));
    }

    #[test]
    fn default_filter_excludes_placeholder_and_reports_counts() {
        let conn = setup();
        add_parcel(&conn, "e1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        add_parcel(
            &conn,
            "p1",
            "incomplete taxroll information",
            "individual",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        let config = test_config(1, "caveat from config", test_bands());
        let page = list_owner_groups(&config, &conn, &GroupListQuery::default()).expect("page");
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 1);
        assert_eq!(page.rows[0].display_name, "ACME LLC");
    }

    #[test]
    fn selecting_placeholder_shows_those_rows() {
        let conn = setup();
        add_parcel(&conn, "e1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        add_parcel(
            &conn,
            "p1",
            "incomplete taxroll information",
            "individual",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        let config = test_config(1, "caveat from config", test_bands());
        let page = list_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                owner_type: Some("placeholder".to_string()),
                apply_default_owner_type_exclusions: Some(true),
                ..GroupListQuery::default()
            },
        )
        .expect("page");
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 1);
        assert_eq!(page.rows[0].display_name, "incomplete taxroll information");
        assert_eq!(page.rows[0].owner_type.as_deref(), Some("placeholder"));
    }

    #[test]
    fn nonprofit_appears_by_default_and_tribal_does_not() {
        let conn = setup();
        add_parcel(
            &conn,
            "e1",
            "ACME LLC",
            "entity",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        add_parcel(
            &conn,
            "n1",
            "YMCA OF GREATER WAUKESHA COUNTY INC",
            "entity",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        add_parcel(
            &conn,
            "t1",
            "FOREST COUNTY POTAWATOMI",
            "individual",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        let config = test_config(1, "caveat from config", test_bands());
        let page = list_owner_groups(&config, &conn, &GroupListQuery::default()).expect("page");
        assert_eq!(page.total_before_filter, 3);
        assert_eq!(page.total_after_filter, 2);
        let names: Vec<&str> = page
            .rows
            .iter()
            .map(|row| row.display_name.as_str())
            .collect();
        assert!(names.contains(&"ACME LLC"));
        assert!(names.contains(&"YMCA OF GREATER WAUKESHA COUNTY INC"));
        assert!(!names.contains(&"FOREST COUNTY POTAWATOMI"));
        let ymca = page
            .rows
            .iter()
            .find(|row| row.display_name == "YMCA OF GREATER WAUKESHA COUNTY INC")
            .expect("ymca");
        assert_eq!(ymca.owner_type.as_deref(), Some("nonprofit"));
        let options = list_filter_options(&config, &conn).expect("options");
        assert!(options.owner_types.contains(&"nonprofit".to_string()));
        assert!(options.owner_types.contains(&"tribal".to_string()));
        assert!(options
            .default_excluded_owner_types
            .contains(&"tribal".to_string()));
        assert!(!options
            .default_excluded_owner_types
            .contains(&"nonprofit".to_string()));
    }

    #[test]
    fn selecting_government_shows_those_rows() {
        let conn = setup();
        add_parcel(&conn, "e1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        add_parcel(
            &conn,
            "g1",
            "CITY OF OCONOMOWOC",
            "government",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        let config = test_config(1, "caveat from config", test_bands());
        let page = list_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                owner_type: Some("government".to_string()),
                apply_default_owner_type_exclusions: Some(true),
                ..GroupListQuery::default()
            },
        )
        .expect("page");
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 1);
        assert_eq!(page.rows[0].display_name, "CITY OF OCONOMOWOC");
        assert_eq!(page.rows[0].owner_type.as_deref(), Some("government"));
    }

    #[test]
    fn filters_combine_and_name_uses_normalised_key() {
        let conn = setup();
        add_parcel(&conn, "a1", "BIELINSKI HOMES INC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        add_parcel(&conn, "a2", "BIELINSKI HOMES, INC.", "entity", "NBC", 0.0, "2026-08-10", "55133");
        add_parcel(&conn, "b1", "SOLO TRUST", "trust", "WAKC", 100.0, "2026-08-10", "55133");
        let config = test_config(1, "caveat from config", test_bands());
        let page = list_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                municipality: Some("NBC".to_string()),
                owner_type: Some("entity".to_string()),
                name_contains: Some("bielinski homes, inc.".to_string()),
                min_parcels: Some(2),
                min_improvement_value_zero: Some(2),
                band: Some(BandFilter::Band {
                    label: "few".to_string(),
                }),
                ..GroupListQuery::default()
            },
        )
        .expect("page");
        assert_eq!(page.total_before_filter, 2);
        assert_eq!(page.total_after_filter, 1);
        assert_eq!(page.rows[0].group_key, "BIELINSKI HOMES INC");
        assert!(page.predicate_chain.contains("group has muni_code = NBC"));
        assert!(page.predicate_chain.contains("owner_type = entity"));
        assert!(page.predicate_chain.contains("owner_group_key contains BIELINSKI HOMES INC"));
    }

    #[test]
    fn out_of_bands_and_unknown_are_states_not_labels() {
        let conn = setup();
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value,
                 county_fips, value_as_of
             ) VALUES (1, 'n1', 'NULL VACANT LLC', 'entity', 'WAKC', NULL, '55133', '2026-08-10')",
            [],
        )
        .expect("null improvement");
        crate::current::mirror_latest_entity(&conn).expect("mirror");
        let gapped = vec![
            radar_config::attributes::Band {
                min: Some(0.0),
                max: Some(0.0),
                label: "zero".to_string(),
                favourability: radar_config::Favourability::Neutral,
            },
            radar_config::attributes::Band {
                min: Some(10.0),
                max: None,
                label: "many".to_string(),
                favourability: radar_config::Favourability::Favourable,
            },
        ];
        add_parcel(
            &conn,
            "g1",
            "GAP LLC",
            "entity",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        add_parcel(
            &conn,
            "g2",
            "GAP LLC",
            "entity",
            "WAKC",
            0.0,
            "2026-08-10",
            "55133",
        );
        let config = test_config(1, "caveat from config", gapped);
        let unknown = list_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                band: Some(BandFilter::Unknown),
                ..GroupListQuery::default()
            },
        )
        .expect("unknown");
        assert_eq!(unknown.total_after_filter, 1);
        assert_eq!(unknown.rows[0].band, BandState::Unknown);
        assert_eq!(unknown.rows[0].improvement_value_zero_count, None);

        let outside = list_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                band: Some(BandFilter::OutOfBands),
                ..GroupListQuery::default()
            },
        )
        .expect("outside");
        assert_eq!(outside.total_after_filter, 1);
        match &outside.rows[0].band {
            BandState::OutOfBands { value } => assert_eq!(*value, 2.0),
            other => panic!("expected OutOfBands, got {other:?}"),
        }
    }

    #[test]
    fn missing_zone_assignment_is_reported() {
        let conn = setup();
        add_parcel(&conn, "a1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        let mut config = test_config(1, "caveat from config", test_bands());
        config.zones = Zones {
            assignments: vec![ZoneAssignment {
                capability_key: "other".to_string(),
                zone: Zone::Inside,
                rationale: "x".to_string(),
                assigned_on: "2026-01-01".to_string(),
                expires_on: "2026-12-31".to_string(),
                display_caveat: None,
            }],
        };
        let page = list_owner_groups(&config, &conn, &GroupListQuery::default()).expect("page");
        assert!(!page.caveat.assignment_found);
        assert_eq!(page.caveat.text, None);
        assert_eq!(page.limit, DEFAULT_PAGE_LIMIT);
    }

    #[test]
    fn filter_options_come_from_config_and_lookup() {
        let conn = setup();
        add_parcel(&conn, "a1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        add_parcel(&conn, "a2", "PAT SMITH", "individual", "NBC", 1.0, "2026-08-10", "55133");
        let config = test_config(1, "caveat from config", test_bands());
        let options = list_filter_options(&config, &conn).expect("options");
        assert_eq!(options.county_fips, config.manifest.county_fips);
        assert_eq!(options.county_name, config.manifest.county_name);
        assert_eq!(options.municipalities.len(), 2);
        assert!(options.owner_types.contains(&"entity".to_string()));
        assert!(options.owner_types.contains(&"individual".to_string()));
        let labels: Vec<String> = options
            .bands
            .iter()
            .filter_map(|band| match band {
                BandFilter::Band { label } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["zero", "few", "many"]);
        assert!(options.bands.iter().any(|band| matches!(band, BandFilter::OutOfBands)));
        assert!(options.bands.iter().any(|band| matches!(band, BandFilter::Unknown)));
    }

    #[test]
    fn parcels_behind_a_group_keep_nulls_as_none() {
        let conn = setup();
        add_parcel(&conn, "a1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        conn.execute(
            "UPDATE current_parcels SET assessed_value = NULL, land_value = NULL, site_address = NULL
             WHERE taxkey = 'a1'",
            [],
        )
        .expect("nulls");
        let config = test_config(1, "caveat from config", test_bands());
        let parcels = list_group_parcels(&config, &conn, "ACME LLC").expect("parcels");
        assert_eq!(parcels.len(), 1);
        assert_eq!(parcels[0].assessed_value, None);
        assert_eq!(parcels[0].site_address, None);
        assert_eq!(parcels[0].improvement_value, Some(0.0));
        assert_eq!(parcels[0].municipality.as_deref(), Some("Waukesha City"));
    }

    #[test]
    fn export_writes_filtered_rows_and_caveat() {
        let conn = setup();
        add_parcel(&conn, "a1", "ACME LLC", "entity", "WAKC", 0.0, "2026-08-10", "55133");
        add_parcel(&conn, "b1", "BETA TRUST", "trust", "WAKC", 0.0, "2026-08-10", "55133");
        let config = test_config(1, "caveat from config", test_bands());
        let dir = std::env::temp_dir().join(format!(
            "radar_list_export_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let csv_path = dir.join("groups.csv");
        let result = export_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                owner_type: Some("entity".to_string()),
                ..GroupListQuery::default()
            },
            &csv_path,
        )
        .expect("export");
        assert_eq!(result.rows_exported, 1);
        let body = std::fs::read_to_string(&csv_path).expect("csv");
        assert!(body.contains("ACME LLC"));
        assert!(!body.contains("BETA TRUST"));
        let provenance = std::fs::read_to_string(dir.join("groups.provenance.txt")).expect("prov");
        assert!(provenance.contains("caveat from config"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_store_owner_group_commands_return_ok() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let db = root.join("data/radar.db");
        let config_dir = root.join("config/dr_data_icp");
        assert!(
            db.exists(),
            "live store missing at {}",
            db.display()
        );
        let mut conn = Connection::open(&db).expect("open live store");
        crate::migrate::configure_connection(&conn).expect("sqlite pragmas");
        migrate(&mut conn).expect("migrate live store");
        crate::current::rebuild_current_parcels(&conn, None).expect("rebuild current_parcels");
        let config = load(&config_dir).expect("load config");

        let started = Instant::now();
        let options = list_filter_options(&config, &conn).expect("owner_group_filter_options");
        println!(
            "TASK5 owner_group_filter_options Ok in {:?} owner_types={:?} municipalities={} bands={}",
            started.elapsed(),
            options.owner_types,
            options.municipalities.len(),
            options.bands.len()
        );

        let started = Instant::now();
        let page = list_owner_groups(
            &config,
            &conn,
            &GroupListQuery {
                offset: Some(0),
                limit: Some(500_000),
                ..GroupListQuery::default()
            },
        )
        .expect("list_owner_groups");
        println!(
            "TASK5 list_owner_groups Ok in {:?} total_before_filter={} total_after_filter={} first_page_cap={}",
            started.elapsed(),
            page.total_before_filter,
            page.total_after_filter,
            page.rows.len().min(100)
        );

        let entity = page
            .rows
            .iter()
            .filter(|row| row.owner_type.as_deref() == Some("entity"))
            .count();
        let entity_zero_six = page
            .rows
            .iter()
            .filter(|row| {
                row.owner_type.as_deref() == Some("entity")
                    && row.improvement_value_zero_count.unwrap_or(0) >= 6
            })
            .count();
        println!("TASK5 owner_type entity count={entity}");
        println!(
            "TASK5 entity improvement_value_zero_count>=6 count={entity_zero_six}"
        );

        let mut bands: BTreeMap<String, i64> = BTreeMap::new();
        for row in &page.rows {
            let key = match &row.band {
                BandState::Band {
                    label,
                    favourability,
                    ..
                } => format!("band:{label}:{favourability}"),
                BandState::OutOfBands { value } => format!("out_of_bands:{value}"),
                BandState::Unknown => "unknown".to_string(),
            };
            *bands.entry(key).or_insert(0) += 1;
        }
        println!("TASK5 band distribution={bands:?}");

        println!("TASK5 first five unfiltered rows:");
        for row in page.rows.iter().take(5) {
            println!(
                "  group_key={:?} display_name={:?} member_name_count={} total_parcels={:?} improvement_value_zero_count={:?} municipality_count={} mixed_owner_type={} owner_type={:?} band={:?} value_as_of={:?}",
                row.group_key,
                row.display_name,
                row.member_name_count,
                row.total_parcels,
                row.improvement_value_zero_count,
                row.municipality_count,
                row.mixed_owner_type,
                row.owner_type,
                row.band,
                row.value_as_of
            );
        }

        assert!(page.total_before_filter > 0);
        assert!(!options.municipalities.is_empty());
        assert!(!options.owner_types.is_empty());
        assert!(!options.bands.is_empty());
        assert_eq!(
            page.rows.len().min(100),
            100.min(page.total_after_filter as usize)
        );
    }

    #[test]
    fn live_store_list_owner_groups_wall_clock() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let db = root.join("data/radar.db");
        let config_dir = root.join("config/dr_data_icp");
        assert!(db.exists(), "live store missing at {}", db.display());
        let mut conn = Connection::open(&db).expect("open live store");
        crate::migrate::configure_connection(&conn).expect("sqlite pragmas");
        let config = load(&config_dir).expect("load config");
        let county = config.manifest.county_fips.as_str();
        println!("MEASURE cwd is DrData_ICP_Lead_radar, not Karl-Intel-Desktop, not MVP_Hub");
        match super::explain_owner_group_list_plan(&config, &conn) {
            Ok(plan) => {
                for line in plan {
                    println!("MEASURE plan before_index {line}");
                }
            }
            Err(err) => println!("MEASURE plan before_index error {err}"),
        }
        migrate(&mut conn).expect("migrate live store");
        let all_parcels: i64 = conn
            .query_row("SELECT COUNT(*) FROM current_parcels", [], |row| row.get(0))
            .expect("all parcel count");
        let parcels: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1",
                [county],
                |row| row.get(0),
            )
            .expect("parcel count");
        let named: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_parcels WHERE county_fips = ?1 AND owner_name IS NOT NULL",
                [county],
                |row| row.get(0),
            )
            .expect("named count");
        println!("MEASURE current_parcels all_rows={all_parcels} county_rows={parcels} named_county_rows={named}");
        println!(
            "MEASURE list_owner_groups path=SQL GROUP BY owner_name on current_parcels covering index, then UDF map, then GROUP BY group_key, LIMIT 100. Rust materialises only the page rows."
        );
        match super::explain_owner_group_list_plan(&config, &conn) {
            Ok(plan) => {
                for line in plan {
                    println!("MEASURE plan after_index {line}");
                }
            }
            Err(err) => println!("MEASURE plan after_index error {err}"),
        }
        for run in 1..=3 {
            let started = Instant::now();
            let options = list_filter_options(&config, &conn).expect("options");
            println!(
                "MEASURE owner_group_filter_options run={run} wall_ms={} municipalities={}",
                started.elapsed().as_millis(),
                options.municipalities.len()
            );
        }
        let query = GroupListQuery {
            apply_default_owner_type_exclusions: Some(true),
            ..GroupListQuery::default()
        };
        for run in 1..=3 {
            let started = Instant::now();
            let page = list_owner_groups(&config, &conn, &query).expect("list");
            println!(
                "MEASURE list_owner_groups run={run} wall_ms={} rust_rows={} total_before_filter={} total_after_filter={}",
                started.elapsed().as_millis(),
                page.rows.len(),
                page.total_before_filter,
                page.total_after_filter
            );
            assert_eq!(page.rows.len(), 100.min(page.total_after_filter as usize));
        }
    }
}
