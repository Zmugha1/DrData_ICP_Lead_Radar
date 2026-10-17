use std::path::PathBuf;

use radar_config::{RadarConfig, SourceStatus};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::StoreError;

// Availability is derived from the loaded source registry, not stored.
// When a source moves from Scoped to Live the page picks that up on
// the next read. A stored availability column would go stale the day
// the work is done.
//
// Source status and detection status are different facts. A Live
// source with no detection produces nothing. Showing only source
// status would tell a user a signal is available when it is not.
// The same failure shape as a null rendering as zero.

pub const SOURCE_NOT_REGISTERED: &str = "source not registered";
pub const DETECTION_BUILT: &str = "built";
pub const DETECTION_NOT_BUILT: &str = "not built";
pub const DETECTION_NOT_APPLICABLE: &str = "not applicable";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalCatalogueQuery {
    pub vertical_key: Option<String>,
    pub signal_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VerticalCatalogRow {
    pub vertical_key: String,
    pub label: String,
    pub tier: String,
    pub description: Option<String>,
    pub confirmed_in_county_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignalCatalogRow {
    pub signal_key: String,
    pub label: String,
    pub description: String,
    pub source_id: String,
    pub detection_method: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignalDirectionRow {
    pub signal_key: String,
    pub signal_label: String,
    pub description: String,
    pub source_id: String,
    pub detection_method: String,
    pub source_status: String,
    pub detection_built: String,
    pub vertical_key: String,
    pub vertical_label: String,
    pub tier: String,
    pub direction: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignalCataloguePage {
    pub verticals: Vec<VerticalCatalogRow>,
    pub signals: Vec<SignalCatalogRow>,
    pub rows: Vec<SignalDirectionRow>,
}

pub fn signal_catalogue(
    config: &RadarConfig,
    conn: &Connection,
    query: &SignalCatalogueQuery,
) -> Result<SignalCataloguePage, StoreError> {
    validate_vertical_catalog_tiers(config, conn)?;
    Ok(SignalCataloguePage {
        verticals: list_verticals(conn)?,
        signals: list_signals(conn)?,
        rows: list_directions(config, conn, query)?,
    })
}

/// Every vertical_catalog.tier value must be a tier_key in tiers.toml.
/// This is the store-side half of V33. Config cannot foreign-key SQLite
/// to a TOML file, so the check runs when the catalogue is read.
pub fn validate_vertical_catalog_tiers(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<(), StoreError> {
    let mut stmt = conn.prepare("SELECT vertical_key, tier FROM vertical_catalog")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (vertical_key, tier) = row?;
        if !config.tiers.contains_key(&tier) {
            return Err(StoreError::Catalog(format!(
                "vertical_catalog.{vertical_key} has tier {tier} which is not a tier_key in tiers.toml"
            )));
        }
    }
    Ok(())
}

fn list_verticals(conn: &Connection) -> Result<Vec<VerticalCatalogRow>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT v.vertical_key, v.label, v.tier, e.description,
                COALESCE(
                    (SELECT c.group_count
                     FROM vertical_catalog_county_counts c
                     WHERE c.vertical_key = v.vertical_key
                     ORDER BY c.measured_on DESC
                     LIMIT 1),
                    e.confirmed_in_county_count
                )
         FROM vertical_catalog v
         LEFT JOIN vertical_catalog_elicited e ON e.vertical_key = v.vertical_key
         ORDER BY v.tier, v.label",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(VerticalCatalogRow {
            vertical_key: row.get(0)?,
            label: row.get(1)?,
            tier: row.get(2)?,
            description: row.get(3)?,
            confirmed_in_county_count: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn list_signals(conn: &Connection) -> Result<Vec<SignalCatalogRow>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT signal_key, label, description, source_id, detection_method
         FROM signal_catalog
         ORDER BY signal_key",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SignalCatalogRow {
            signal_key: row.get(0)?,
            label: row.get(1)?,
            description: row.get(2)?,
            source_id: row.get(3)?,
            detection_method: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn list_directions(
    config: &RadarConfig,
    conn: &Connection,
    query: &SignalCatalogueQuery,
) -> Result<Vec<SignalDirectionRow>, StoreError> {
    let vertical_key = nonempty(query.vertical_key.as_deref());
    let signal_key = nonempty(query.signal_key.as_deref());
    let sql = "SELECT s.signal_key, s.label, s.description, s.source_id, s.detection_method,
                      v.vertical_key, v.label, v.tier, d.direction, d.rationale
               FROM signal_direction d
               INNER JOIN signal_catalog s ON s.signal_key = d.signal_key
               INNER JOIN vertical_catalog v ON v.vertical_key = d.vertical_key
               WHERE (?1 IS NULL OR d.vertical_key = ?1)
                 AND (?2 IS NULL OR d.signal_key = ?2)
               ORDER BY s.signal_key, v.label";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params![vertical_key, signal_key], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            signal_key,
            signal_label,
            description,
            source_id,
            detection_method,
            vertical_key,
            vertical_label,
            tier,
            direction,
            rationale,
        ) = row?;
        out.push(SignalDirectionRow {
            source_status: source_status_label(config, &source_id),
            detection_built: detection_built_status(&signal_key, &detection_method),
            signal_key,
            signal_label,
            description,
            source_id,
            detection_method,
            vertical_key,
            vertical_label,
            tier,
            direction,
            rationale,
        });
    }
    Ok(out)
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

fn source_status_label(config: &RadarConfig, source_id: &str) -> String {
    match config
        .sources
        .sources
        .iter()
        .find(|source| source.source_id == source_id)
    {
        Some(source) => match source.status {
            SourceStatus::Live => "Live".to_string(),
            SourceStatus::Profiled => "Profiled".to_string(),
            SourceStatus::Scoped => "Scoped".to_string(),
            SourceStatus::Refused => "Refused".to_string(),
        },
        None => SOURCE_NOT_REGISTERED.to_string(),
    }
}

// Detection is built when crates/radar-store/src/detection/<signal_key>.rs
// exists. That file is the named module for the detector. Manual
// upload and external file have nothing to build in this sense, so
// they are not applicable. compare_load_runs and
// print_plan_commission_match are fetch assertions, not these
// detectors, and are not consulted.
fn detector_module_path(signal_key: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("detection")
        .join(format!("{signal_key}.rs"))
}

pub fn detection_built_status(signal_key: &str, detection_method: &str) -> String {
    match detection_method {
        "manual upload" | "external file" => DETECTION_NOT_APPLICABLE.to_string(),
        _ => {
            if detector_module_path(signal_key).is_file() {
                DETECTION_BUILT.to_string()
            } else {
                DETECTION_NOT_BUILT.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use radar_config::sources::{SourceEntry, SourceFamily, SourceKind, SourceStatus};
    use rusqlite::Connection;

    use super::{
        detection_built_status, detector_module_path, signal_catalogue, SignalCatalogueQuery,
        DETECTION_BUILT, DETECTION_NOT_APPLICABLE, DETECTION_NOT_BUILT, SOURCE_NOT_REGISTERED,
    };
    use crate::migrate::migrate;
    use crate::rank::tests::{test_bands, test_config};

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    fn source(id: &str, status: SourceStatus) -> SourceEntry {
        SourceEntry {
            source_id: id.to_string(),
            display_name: id.to_string(),
            family: SourceFamily::PropertyEvent,
            kind: SourceKind::Csv,
            status,
            cadence_days: None,
            publication_lag_days: None,
            lag_measured: false,
            url: None,
            url_verified: false,
            requires_key: false,
            attribution: None,
            notes: None,
        }
    }

    #[test]
    fn live_source_reports_live() {
        let conn = open();
        let mut config = test_config(1, "caveat", test_bands());
        config
            .sources
            .sources
            .push(source("src-waukesha-parcels", SourceStatus::Live));
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("new_parcel".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        assert!(!page.rows.is_empty());
        assert!(page.rows.iter().all(|row| row.source_status == "Live"));
        assert!(page
            .rows
            .iter()
            .all(|row| row.source_id == "src-waukesha-parcels"));
    }

    #[test]
    fn missing_source_id_reports_not_registered() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("new_parcel".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        assert!(!page.rows.is_empty());
        assert!(page
            .rows
            .iter()
            .all(|row| row.source_status == SOURCE_NOT_REGISTERED));
        assert!(!page.rows.iter().any(|row| row.source_status.is_empty()));
    }

    #[test]
    fn ask_renders_as_ask_not_blank() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("owner_name_amended".to_string()),
                vertical_key: Some("custom_homebuilder".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].direction, "ask");
        assert!(!page.rows[0].direction.is_empty());
        assert!(!page.rows[0].rationale.is_empty());
    }

    #[test]
    fn by_signal_improvement_zero_to_value_has_more_than_one_direction() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("improvement_zero_to_value".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        let directions: std::collections::BTreeSet<&str> =
            page.rows.iter().map(|row| row.direction.as_str()).collect();
        assert!(
            directions.len() > 1,
            "expected a reversal, got {directions:?}"
        );
        assert!(directions.contains("favourable"));
        assert!(directions.contains("unfavourable"));
        let builder = page
            .rows
            .iter()
            .find(|row| row.vertical_key == "custom_homebuilder")
            .expect("builder");
        let landscape = page
            .rows
            .iter()
            .find(|row| row.vertical_key == "landscape_design_build")
            .expect("landscape");
        assert_eq!(builder.direction, "unfavourable");
        assert_eq!(landscape.direction, "favourable");
    }

    #[test]
    fn improvement_zero_to_value_description_names_republish_cadence() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("improvement_zero_to_value".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        assert!(!page.rows.is_empty());
        let body = &page.rows[0].description;
        assert!(body.contains("produces nothing unless the source has republished"));
        assert!(body.contains("however often the county republishes"));
        assert!(body.contains("Milwaukee publishes no export date"));
    }

    #[test]
    fn plan_commission_item_uses_agenda_source_and_reports_scoped() {
        let conn = open();
        let mut config = test_config(1, "caveat", test_bands());
        config.sources.sources.push(source(
            "src-municipal-plan-commission-agendas",
            SourceStatus::Scoped,
        ));
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("plan_commission_item".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        assert!(!page.rows.is_empty());
        assert!(page.rows.iter().all(|row| {
            row.source_id == "src-municipal-plan-commission-agendas"
                && row.source_status == "Scoped"
                && row.detection_built == DETECTION_NOT_APPLICABLE
        }));
        let row = &page.rows[0];
        println!(
            "plan_commission_item source={} source_status={} detection_built={} method={}",
            row.source_id, row.source_status, row.detection_built, row.detection_method
        );
    }

    #[test]
    fn run_comparison_on_live_source_is_live_and_built() {
        let conn = open();
        let mut config = test_config(1, "caveat", test_bands());
        config
            .sources
            .sources
            .push(source("src-waukesha-parcels", SourceStatus::Live));
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("improvement_zero_to_value".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        assert!(!page.rows.is_empty());
        for row in &page.rows {
            assert_eq!(row.source_status, "Live");
            assert_eq!(row.detection_built, DETECTION_BUILT);
            assert_ne!(row.source_status, row.detection_built);
            let encoded = serde_json::to_value(row).expect("json");
            assert!(encoded.get("availability").is_none());
            assert!(encoded.get("source_status").is_some());
            assert!(encoded.get("detection_built").is_some());
        }
        let row = &page.rows[0];
        println!(
            "improvement_zero_to_value source={} source_status={} detection_built={} method={}",
            row.source_id, row.source_status, row.detection_built, row.detection_method
        );
    }

    #[test]
    fn manual_upload_reports_detection_not_applicable() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        let page = signal_catalogue(
            &config,
            &conn,
            &SignalCatalogueQuery {
                signal_key: Some("plan_commission_item".to_string()),
                ..SignalCatalogueQuery::default()
            },
        )
        .expect("catalogue");
        assert!(!page.rows.is_empty());
        assert!(page
            .rows
            .iter()
            .all(|row| row.detection_method == "manual upload"
                && row.detection_built == DETECTION_NOT_APPLICABLE));
    }

    #[test]
    fn no_signal_reports_a_merged_availability_value() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        let page = signal_catalogue(&config, &conn, &SignalCatalogueQuery::default())
            .expect("catalogue");
        assert!(!page.rows.is_empty());
        for row in &page.rows {
            let encoded = serde_json::to_value(row).expect("json");
            assert!(encoded.get("availability").is_none());
            assert!(encoded.get("source_status").is_some());
            assert!(encoded.get("detection_built").is_some());
        }
    }

    #[test]
    fn detection_status_follows_method_and_named_module() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        let page = signal_catalogue(&config, &conn, &SignalCatalogueQuery::default())
            .expect("catalogue");
        for signal in &page.signals {
            let path = detector_module_path(&signal.signal_key);
            let derived = detection_built_status(&signal.signal_key, &signal.detection_method);
            println!(
                "detection check {} method={} module_exists={} path={} => {}",
                signal.signal_key,
                signal.detection_method,
                path.is_file(),
                path.display(),
                derived
            );
            match signal.detection_method.as_str() {
                "manual upload" | "external file" => {
                    assert_eq!(derived, DETECTION_NOT_APPLICABLE);
                }
                "run comparison" | "nightly diff" | "not yet determined" => {
                    match signal.signal_key.as_str() {
                        "improvement_zero_to_value"
                        | "new_liquor_licence"
                        | "liquor_licence_ended"
                        | "licensee_changed"
                        | "occupancy_new_business" => {
                            assert!(path.is_file());
                            assert_eq!(derived, DETECTION_BUILT);
                        }
                        _ => {
                            assert!(!path.is_file());
                            assert_eq!(derived, DETECTION_NOT_BUILT);
                        }
                    }
                }
                other => panic!("unexpected detection_method {other}"),
            }
        }
    }

    #[test]
    fn every_vertical_tier_matches_tiers_toml() {
        let conn = open();
        let config = test_config(1, "caveat", test_bands());
        super::validate_vertical_catalog_tiers(&config, &conn).expect("tiers match");
        let page = signal_catalogue(&config, &conn, &SignalCatalogueQuery::default())
            .expect("catalogue");
        assert_eq!(page.verticals.len(), 19);
        for vertical in &page.verticals {
            assert!(
                config.tiers.contains_key(&vertical.tier),
                "vertical {} tier {}",
                vertical.vertical_key,
                vertical.tier
            );
        }
        let realtor = page
            .verticals
            .iter()
            .find(|row| row.vertical_key == "realtor")
            .expect("realtor");
        assert!(realtor.description.is_none());
        let builder = page
            .verticals
            .iter()
            .find(|row| row.vertical_key == "custom_homebuilder")
            .expect("builder");
        assert_eq!(builder.confirmed_in_county_count, Some(136));
        assert!(builder
            .description
            .as_ref()
            .expect("elicited")
            .contains("Guaranteed or fixed pricing"));
    }

    #[test]
    fn catalog_tier_unknown_to_tiers_toml_is_refused() {
        let conn = open();
        conn.execute(
            "UPDATE vertical_catalog SET tier = '9' WHERE vertical_key = 'realtor'",
            [],
        )
        .expect("mutate fixture");
        let config = test_config(1, "caveat", test_bands());
        let err = signal_catalogue(&config, &conn, &SignalCatalogueQuery::default())
            .expect_err("unknown tier");
        let text = err.to_string();
        assert!(text.contains("realtor"), "{text}");
        assert!(text.contains("9"), "{text}");
    }
}
