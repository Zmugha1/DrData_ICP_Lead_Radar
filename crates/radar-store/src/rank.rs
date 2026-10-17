use radar_config::{Direction, Favourability, RadarConfig};
use rusqlite::Connection;

use crate::evaluate::{evaluate_attribute, AttributeEvaluation, EvaluateError};
use crate::groups::{fetch_owner_groups, OwnerGroup};
use crate::StoreError;

// trigger_status is blank by design and becomes real only when a
// trigger source reaches status Live. Every trigger in config
// currently carries wired = false. The value is not computed.
pub const TRIGGER_STATUS_UNWIRED: &str = "unknown";

// Outcomes are a record of actions against a subject. rank_groups must
// not read the outcomes table. Recording is the whole job until there
// are months of data. This data will eventually inform the Below Zone
// assignment prospect_ranking_by_likelihood_to_buy. Until then, nothing
// here may use outcomes to rank, score or suggest.

// First cut of the capacity rule: one month of first meetings is
// first_meetings_per_week times four. No outcome evidence behind it.
const WEEKS_IN_WORKING_SET: u32 = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct RankedGroup {
    pub rank: i64,
    pub group_key: String,
    pub display_name: String,
    pub member_name_count: i64,
    pub total_parcels: Option<i64>,
    pub improvement_value_zero_count: Option<i64>,
    pub band_label: Option<String>,
    pub band_favourability: Option<Favourability>,
    pub municipality_count: i64,
    pub trigger_status: String,
    pub in_working_set: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RankError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Evaluate(#[from] EvaluateError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub fn rank_groups(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<Vec<RankedGroup>, RankError> {
    let groups = fetch_owner_groups(conn, &config.owner_type)?;
    let vacant_key = config
        .attributes
        .attributes
        .iter()
        .find(|attribute| attribute.direction == Direction::Band)
        .map(|attribute| attribute.key.as_str());

    let mut scored: Vec<(OwnerGroup, Option<String>, Option<Favourability>)> = Vec::new();
    for group in groups {
        if group.owner_type.as_deref() != Some("entity") || group.mixed_owner_type {
            continue;
        }
        let (band_label, band_favourability) = match vacant_key {
            Some(key) => match evaluate_attribute(config, key, &group)? {
                AttributeEvaluation::Band {
                    label,
                    favourability,
                    ..
                } => (Some(label), Some(favourability)),
                AttributeEvaluation::OutOfBands { .. } | AttributeEvaluation::Unknown => {
                    (None, None)
                }
                AttributeEvaluation::Value { .. } => (None, None),
            },
            None => (None, None),
        };
        scored.push((group, band_label, band_favourability));
    }

    scored.sort_by(|a, b| {
        favourability_order(a.2.as_ref())
            .cmp(&favourability_order(b.2.as_ref()))
            .then(b.0.improvement_value_zero_count.cmp(&a.0.improvement_value_zero_count))
            .then(b.0.total_parcels.cmp(&a.0.total_parcels))
            .then(a.0.group_key.cmp(&b.0.group_key))
    });

    let working_set_size =
        (config.work_scope.capacity.first_meetings_per_week.saturating_mul(WEEKS_IN_WORKING_SET))
            as usize;

    let ranked = scored
        .into_iter()
        .enumerate()
        .map(|(index, (group, band_label, band_favourability))| RankedGroup {
            rank: (index as i64) + 1,
            group_key: group.group_key,
            display_name: group.display_name,
            member_name_count: group.member_name_count,
            total_parcels: group.total_parcels,
            improvement_value_zero_count: group.improvement_value_zero_count,
            band_label,
            band_favourability,
            municipality_count: group.municipality_count,
            trigger_status: TRIGGER_STATUS_UNWIRED.to_string(),
            in_working_set: index < working_set_size,
        })
        .collect();
    Ok(ranked)
}

pub(crate) fn favourability_order(value: Option<&Favourability>) -> u8 {
    match value {
        Some(Favourability::Favourable) => 0,
        Some(Favourability::Neutral) => 1,
        Some(Favourability::Unfavourable) => 2,
        None => 3,
    }
}

#[cfg(test)]
pub mod tests {
    use radar_config::assignments::AssignmentsFile;
    use radar_config::attributes::{Attribute, Attributes, Band, Direction, Favourability, ValueType};
    use radar_config::boundaries::{AbsenceRule, AbsenceRuleKind, Boundaries};
    use radar_config::coverage::{Coverage, CoverageEntry, CoverageLevel, PublicationLag};
    use radar_config::entity::{Attachment, Entity, FieldMap, JoinMethod, Layer, LayerRole};
    use radar_config::manifest::{EntityUniverse, Manifest};
    use radar_config::series::SeriesFile;
    use radar_config::sources::{SourceEntry, SourceFamily, SourceKind, SourceStatus, SourcesFile};
    use radar_config::triggers::{Trigger, Triggers};
    use radar_config::vocabulary::Vocabulary;
    use radar_config::work_scope::{Capacity, Job, WorkScope};
    use radar_config::zones::{Zone, ZoneAssignment, Zones};
    use radar_config::RadarConfig;
    use rusqlite::Connection;

    use super::{rank_groups, TRIGGER_STATUS_UNWIRED};
    use crate::migrate::migrate;

    fn stub_entity_layers() -> Vec<Layer> {
        vec![Layer {
            role: LayerRole::Primary,
            url: "https://example.invalid/primary".to_string(),
            where_clause: "1=1".to_string(),
            page_size: None,
            vintage_field: None,
            field_map: FieldMap {
                taxkey: "TAXKEY".to_string(),
                muni_code: "MUNI".to_string(),
                owner_name: "OWNERNAMEFULL".to_string(),
                site_address: None,
                mail_address_1: None,
                mail_address_2: None,
                assessed_value: None,
                improvement_value: None,
                land_value: None,
                prop_class: None,
                rec_source: None,
            },
        }]
    }

    pub fn test_config(meetings_per_week: u32, caveat: &str, bands: Vec<Band>) -> RadarConfig {
        RadarConfig {
            manifest: Manifest {
                schema_version: "1".to_string(),
                config_version: "test-rank".to_string(),
                vertical_id: "test".to_string(),
                vertical_display_name: "Test".to_string(),
                county_fips: "55133".to_string(),
                county_name: "Waukesha".to_string(),
            entity_universe: EntityUniverse::Parcels,
            pages: radar_config::default_pages(),
        },
            entity: Entity {
                spine_source_id: "parcels".to_string(),
                county_fips: "55133".to_string(),
                natural_key_field: Some("taxkey".to_string()),
                has_stable_key: true,
                spine_coverage_fallback_threshold: None,
                layers: stub_entity_layers(),
                attachments: vec![Attachment {
                    source_id: "muni".to_string(),
                    join_field: "muni_code".to_string(),
                    join_method: JoinMethod::ExactKey,
                    expected_match_rate: 1.0,
                }],
                lookup_source: None,
                collapse_duplicate_geometry: false,
            },
            attributes: Attributes {
                attributes: vec![Attribute {
                    key: "improvement_value_zero_count".to_string(),
                    source_table: "owner_groups".to_string(),
                    source_column: "vacant_lots".to_string(),
                    value_type: ValueType::Numeric,
                    direction: Direction::Band,
                    bands: Some(bands),
                    variance_tested: false,
                    variance_tested_on: None,
                }],
            },
            triggers: Triggers {
                triggers: vec![Trigger {
                    key: "t".to_string(),
                    source_id: "s".to_string(),
                    published: true,
                    wired: false,
                    lag_days: None,
                    hard_stop: None,
                }],
            },
            work_scope: WorkScope {
                jobs: vec![Job {
                    key: "j".to_string(),
                    label: "J".to_string(),
                    ticket_low: 1,
                    ticket_high: 2,
                    description: None,
                }],
                capacity: Capacity {
                    builds_concurrent: 1,
                    first_meetings_per_week: meetings_per_week,
                },
            },
            vocabulary: Vocabulary::from_test_lists(
                vec!["a".to_string()],
                vec!["b".to_string()],
                vec!["c".to_string()],
                vec!["d".to_string()],
            ),
            zones: Zones {
                assignments: vec![ZoneAssignment {
                    capability_key: "prospect_ranking_by_likelihood_to_buy".to_string(),
                    zone: Zone::Below,
                    rationale: "test".to_string(),
                    assigned_on: "2026-01-01".to_string(),
                    expires_on: "2026-12-31".to_string(),
                    display_caveat: Some(caveat.to_string()),
                }],
            },
            boundaries: Boundaries {
                approval_required_by_default: true,
                absence_rules: vec![AbsenceRule {
                    item: "x".to_string(),
                    rule: AbsenceRuleKind::Drop,
                }],
                never_stored: vec!["y".to_string()],
                cloud_extraction_allowed_for: vec!["z".to_string()],
            },
            coverage: Coverage {
                geographies: vec![CoverageEntry {
                    key: "g".to_string(),
                    level: CoverageLevel::Full,
                    note: "n".to_string(),
                }],
                attributes: vec![CoverageEntry {
                    key: "a".to_string(),
                    level: CoverageLevel::Full,
                    note: "n".to_string(),
                }],
                publication_lags: vec![PublicationLag {
                    source_id: "s".to_string(),
                    lag_days: 0,
                    measured: false,
                    measured_from: None,
                }],
            },
            sources: SourcesFile {
                sources: vec![SourceEntry {
                    source_id: "parcels".to_string(),
                    display_name: "p".to_string(),
                    family: SourceFamily::PropertyEvent,
                    kind: SourceKind::Csv,
                    status: SourceStatus::Live,
                    cadence_days: None,
                    publication_lag_days: None,
                    lag_measured: false,
                    url: None,
                    url_verified: false,
                    requires_key: false,
                    attribution: None,
                    notes: None,
                }],
            },
            series: SeriesFile { series: vec![] },
            assignments: AssignmentsFile::from_dr_data_file(),
            owner_type: radar_config::OwnerTypeFile::from_dr_data_file(),
            tiers: radar_config::TiersFile::test_fixture(),
            search_query_templates: None,
        }
    }

    pub fn test_bands() -> Vec<Band> {
        vec![
            Band {
                min: Some(0.0),
                max: Some(0.0),
                label: "zero".to_string(),
                favourability: Favourability::Neutral,
            },
            Band {
                min: Some(1.0),
                max: Some(5.0),
                label: "few".to_string(),
                favourability: Favourability::Neutral,
            },
            Band {
                min: Some(6.0),
                max: None,
                label: "many".to_string(),
                favourability: Favourability::Favourable,
            },
        ]
    }

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at, value_as_of)
             VALUES (1, 'src-test', 'success', '2026-08-10', '2026-08-10')",
            [],
        )
        .expect("run");
        conn
    }

    fn add_group(conn: &Connection, name: &str, vacant: i64, extra_non_vacant: i64) {
        let mut n = 0;
        for _ in 0..vacant {
            n += 1;
            conn.execute(
                "INSERT INTO entities (load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value, county_fips)
                 VALUES (1, ?1, ?2, 'entity', 'WAKC', 0, '55133')",
                rusqlite::params![format!("{name}-{n}"), name],
            )
            .expect("vacant parcel");
            crate::current::mirror_latest_entity(conn).expect("mirror");
        }
        for _ in 0..extra_non_vacant {
            n += 1;
            conn.execute(
                "INSERT INTO entities (load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value, county_fips)
                 VALUES (1, ?1, ?2, 'entity', 'WAKC', 100, '55133')",
                rusqlite::params![format!("{name}-{n}"), name],
            )
            .expect("built parcel");
            crate::current::mirror_latest_entity(conn).expect("mirror");
        }
    }

    #[test]
    fn orders_across_and_within_bands() {
        let conn = setup();
        add_group(&conn, "ALPHA LLC", 10, 0);
        add_group(&conn, "BETA LLC", 8, 12);
        add_group(&conn, "GAMMA LLC", 10, 0);
        add_group(&conn, "DELTA LLC", 0, 20);
        add_group(&conn, "EPSILON LLC", 3, 0);
        let config = test_config(1, "caveat", test_bands());
        let ranked = rank_groups(&config, &conn).expect("rank");
        let keys: Vec<&str> = ranked.iter().map(|row| row.group_key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "ALPHA LLC",
                "GAMMA LLC",
                "BETA LLC",
                "EPSILON LLC",
                "DELTA LLC",
            ]
        );
        assert_eq!(ranked[0].band_favourability, Some(Favourability::Favourable));
        assert_eq!(ranked[3].band_favourability, Some(Favourability::Neutral));
        assert!(ranked.iter().all(|row| row.trigger_status == TRIGGER_STATUS_UNWIRED));
    }

    #[test]
    fn ties_break_on_group_key_and_are_stable() {
        let conn = setup();
        add_group(&conn, "ZETA LLC", 7, 0);
        add_group(&conn, "ETA LLC", 7, 0);
        let config = test_config(1, "caveat", test_bands());
        let first = rank_groups(&config, &conn).expect("rank 1");
        let second = rank_groups(&config, &conn).expect("rank 2");
        let keys_one: Vec<&str> = first.iter().map(|row| row.group_key.as_str()).collect();
        let keys_two: Vec<&str> = second.iter().map(|row| row.group_key.as_str()).collect();
        assert_eq!(keys_one, vec!["ETA LLC", "ZETA LLC"]);
        assert_eq!(keys_one, keys_two);
    }

    #[test]
    fn working_set_uses_config_capacity() {
        let conn = setup();
        for name in ["A LLC", "B LLC", "C LLC", "D LLC", "E LLC", "F LLC"] {
            add_group(&conn, name, 6, 0);
        }
        let meetings = 1;
        let config = test_config(meetings, "caveat", test_bands());
        let ranked = rank_groups(&config, &conn).expect("rank");
        let expected = (meetings * 4) as usize;
        assert_eq!(expected, 4);
        let flagged = ranked.iter().filter(|row| row.in_working_set).count();
        assert_eq!(flagged, expected);
        assert_eq!(ranked.len(), 6);
        assert!(ranked.iter().take(expected).all(|row| row.in_working_set));
        assert!(ranked.iter().skip(expected).all(|row| !row.in_working_set));
    }
}
