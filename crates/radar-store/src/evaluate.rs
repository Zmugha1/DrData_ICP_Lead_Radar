use radar_config::{Direction, Favourability, RadarConfig};

use crate::groups::OwnerGroup;

#[derive(Debug, Clone, PartialEq)]
pub enum AttributeEvaluation {
    Unknown,
    Value { value: f64 },
    Band {
        value: f64,
        label: String,
        favourability: Favourability,
    },
    OutOfBands { value: f64 },
}

#[derive(Debug, thiserror::Error)]
pub enum EvaluateError {
    #[error("unknown attribute key: {0}")]
    UnknownAttribute(String),
}

pub fn evaluate_attribute(
    config: &RadarConfig,
    attribute_key: &str,
    group: &OwnerGroup,
) -> Result<AttributeEvaluation, EvaluateError> {
    let attribute = config
        .attributes
        .attributes
        .iter()
        .find(|attribute| attribute.key == attribute_key)
        .ok_or_else(|| EvaluateError::UnknownAttribute(attribute_key.to_string()))?;

    let Some(value) = measure(group, &attribute.source_column) else {
        return Ok(AttributeEvaluation::Unknown);
    };

    if attribute.direction == Direction::Band {
        let Some(bands) = attribute.bands.as_ref() else {
            return Ok(AttributeEvaluation::OutOfBands { value });
        };
        for band in bands {
            let min_ok = band.min.map(|min| value >= min).unwrap_or(true);
            let max_ok = band.max.map(|max| value <= max).unwrap_or(true);
            if min_ok && max_ok {
                return Ok(AttributeEvaluation::Band {
                    value,
                    label: band.label.clone(),
                    favourability: band.favourability.clone(),
                });
            }
        }
        return Ok(AttributeEvaluation::OutOfBands { value });
    }

    Ok(AttributeEvaluation::Value { value })
}

fn measure(group: &OwnerGroup, source_column: &str) -> Option<f64> {
    match source_column {
        "total_parcels" => group.total_parcels.map(|value| value as f64),
        "vacant_lots" => group.improvement_value_zero_count.map(|value| value as f64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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

    use super::{evaluate_attribute, AttributeEvaluation};
    use crate::groups::OwnerGroup;

    fn group_with_vacant(count: Option<i64>) -> OwnerGroup {
        OwnerGroup {
            group_key: "TEST".to_string(),
            display_name: "TEST".to_string(),
            display_name_tied: false,
            member_name_count: 1,
            total_parcels: Some(10),
            improvement_value_zero_count: count,
            municipality_count: 1,
            owner_type: Some("entity".to_string()),
            mixed_owner_type: false,
            muni_codes: Default::default(),
            value_as_of_min: None,
            value_as_of_max: None,
        }
    }

    fn vacant_attribute(bands: Vec<Band>) -> Attribute {
        Attribute {
            key: "improvement_value_zero_count".to_string(),
            source_table: "owner_groups".to_string(),
            source_column: "vacant_lots".to_string(),
            value_type: ValueType::Numeric,
            direction: Direction::Band,
            bands: Some(bands),
            variance_tested: false,
            variance_tested_on: None,
        }
    }

    fn default_bands() -> Vec<Band> {
        vec![
            Band {
                min: Some(0.0),
                max: Some(0.0),
                label: "No vacant holdings".to_string(),
                favourability: Favourability::Neutral,
            },
            Band {
                min: Some(1.0),
                max: Some(3.0),
                label: "Small holding".to_string(),
                favourability: Favourability::Neutral,
            },
            Band {
                min: Some(4.0),
                max: Some(20.0),
                label: "Active builder".to_string(),
                favourability: Favourability::Favourable,
            },
            Band {
                min: Some(21.0),
                max: None,
                label: "Large operator".to_string(),
                favourability: Favourability::Unfavourable,
            },
        ]
    }

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

    fn config_with_attributes(attributes: Vec<Attribute>) -> RadarConfig {
        RadarConfig {
            manifest: Manifest {
                schema_version: "1".to_string(),
                config_version: "1".to_string(),
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
            attributes: Attributes { attributes },
            triggers: Triggers { triggers: vec![Trigger {
                key: "t".to_string(),
                source_id: "s".to_string(),
                published: true,
                wired: true,
                lag_days: None,
                hard_stop: None,
            }] },
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
                    first_meetings_per_week: 1,
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
                    capability_key: "k".to_string(),
                    zone: Zone::Inside,
                    rationale: "r".to_string(),
                    assigned_on: "2026-01-01".to_string(),
                    expires_on: "2026-12-31".to_string(),
                    display_caveat: None,
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
                sources: vec![
                    SourceEntry {
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
                    },
                    SourceEntry {
                        source_id: "muni".to_string(),
                        display_name: "m".to_string(),
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
                    },
                    SourceEntry {
                        source_id: "s".to_string(),
                        display_name: "s".to_string(),
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
                    },
                ],
            },
            series: SeriesFile { series: vec![] },
            assignments: AssignmentsFile::from_dr_data_file(),
            owner_type: radar_config::OwnerTypeFile::from_dr_data_file(),
            tiers: radar_config::TiersFile::test_fixture(),
            search_query_templates: None,
        }
    }

    fn band_label(eval: AttributeEvaluation) -> String {
        match eval {
            AttributeEvaluation::Band { label, .. } => label,
            other => panic!("expected Band, got {other:?}"),
        }
    }

    #[test]
    fn bands_resolve_from_config() {
        let config = config_with_attributes(vec![vacant_attribute(default_bands())]);
        let key = "improvement_value_zero_count";
        assert_eq!(
            band_label(evaluate_attribute(&config, key, &group_with_vacant(Some(0))).unwrap()),
            "No vacant holdings"
        );
        assert_eq!(
            band_label(evaluate_attribute(&config, key, &group_with_vacant(Some(3))).unwrap()),
            "Small holding"
        );
        assert_eq!(
            band_label(evaluate_attribute(&config, key, &group_with_vacant(Some(4))).unwrap()),
            "Active builder"
        );
        assert_eq!(
            band_label(evaluate_attribute(&config, key, &group_with_vacant(Some(20))).unwrap()),
            "Active builder"
        );
        assert_eq!(
            band_label(evaluate_attribute(&config, key, &group_with_vacant(Some(21))).unwrap()),
            "Large operator"
        );
    }

    #[test]
    fn negative_is_out_of_bands() {
        let config = config_with_attributes(vec![vacant_attribute(default_bands())]);
        let eval = evaluate_attribute(
            &config,
            "improvement_value_zero_count",
            &group_with_vacant(Some(-1)),
        )
        .unwrap();
        assert_eq!(eval, AttributeEvaluation::OutOfBands { value: -1.0 });
    }

    #[test]
    fn null_is_unknown_not_zero() {
        let config = config_with_attributes(vec![vacant_attribute(default_bands())]);
        let eval = evaluate_attribute(
            &config,
            "improvement_value_zero_count",
            &group_with_vacant(None),
        )
        .unwrap();
        assert_eq!(eval, AttributeEvaluation::Unknown);
    }

    #[test]
    fn changing_config_band_changes_label() {
        let mut bands = default_bands();
        let original = config_with_attributes(vec![vacant_attribute(bands.clone())]);
        let key = "improvement_value_zero_count";
        let group = group_with_vacant(Some(3));
        let before = band_label(evaluate_attribute(&original, key, &group).unwrap());
        bands[0].max = Some(5.0);
        let changed = config_with_attributes(vec![vacant_attribute(bands)]);
        let after = band_label(evaluate_attribute(&changed, key, &group).unwrap());
        assert_ne!(before, after);
        assert_eq!(after, "No vacant holdings");
    }
}
