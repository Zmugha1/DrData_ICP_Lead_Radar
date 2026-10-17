use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::assignments::AssignmentsFile;
use crate::owner_type::OwnerTypeFile;
use crate::attributes::{Attribute, Attributes, Band, Direction, Favourability, ValueType};
use crate::boundaries::{AbsenceRule, AbsenceRuleKind, Boundaries};
use crate::coverage::{Coverage, CoverageEntry, CoverageLevel, PublicationLag};
use crate::entity::{Attachment, Entity, FieldMap, JoinMethod, Layer, LayerRole, LookupSource};
use crate::error::ConfigError;
use crate::load::load;
use crate::manifest::{EntityUniverse, Manifest};
use crate::series::{SeriesEntry, SeriesFile};
use crate::session::SessionState;
use crate::sources::{SourceEntry, SourceFamily, SourceKind, SourceStatus, SourcesFile};
use crate::triggers::{Trigger, Triggers};
use crate::validate::{validate, Severity};
use crate::vocabulary::Vocabulary;
use crate::work_scope::{Capacity, Job, WorkScope};
use crate::zones::{Zone, ZoneAssignment, Zones};
use crate::{nav_pages, PageEntry, RadarConfig, TiersFile};

fn sample_field_map() -> FieldMap {
    FieldMap {
        taxkey: "TAXKEY".to_string(),
        muni_code: "MUNI".to_string(),
        owner_name: "OWNERNAMEFULL".to_string(),
        site_address: Some("SITEADRESS".to_string()),
        mail_address_1: Some("PSTLADRESSLINE1".to_string()),
        mail_address_2: Some("PSTLADRESSLINE2".to_string()),
        assessed_value: Some("CNTASSDVALUE".to_string()),
        improvement_value: Some("IMPVALUE".to_string()),
        land_value: Some("LNDVALUE".to_string()),
        prop_class: Some("PROPCLASS".to_string()),
        rec_source: None,
    }
}

fn primary_layer() -> Layer {
    Layer {
        role: LayerRole::Primary,
        url: "https://example.invalid/primary".to_string(),
        where_clause: "1=1".to_string(),
        page_size: None,
        vintage_field: Some("Export_Date".to_string()),
        field_map: sample_field_map(),
    }
}

fn fallback_layer() -> Layer {
    Layer {
        role: LayerRole::Fallback,
        url: "https://example.invalid/fallback".to_string(),
        where_clause: "1=1".to_string(),
        page_size: None,
        vintage_field: Some("Export_Date".to_string()),
        field_map: sample_field_map(),
    }
}

fn live_source(source_id: &str) -> SourceEntry {
    SourceEntry {
        source_id: source_id.to_string(),
        display_name: source_id.to_string(),
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
    }
}

fn valid_config() -> RadarConfig {
    RadarConfig {
        manifest: Manifest {
            schema_version: "1".to_string(),
            config_version: "1".to_string(),
            vertical_id: "test".to_string(),
            vertical_display_name: "Test".to_string(),
            county_fips: "06001".to_string(),
            county_name: "Alameda".to_string(),
            entity_universe: EntityUniverse::Parcels,
            pages: crate::default_pages(),
        },
            entity: Entity {
            spine_source_id: "parcels".to_string(),
            county_fips: "06001".to_string(),
            natural_key_field: Some("apn".to_string()),
            has_stable_key: true,
            spine_coverage_fallback_threshold: None,
            layers: vec![primary_layer()],
            attachments: vec![Attachment {
                source_id: "sales".to_string(),
                join_field: "apn".to_string(),
                join_method: JoinMethod::ExactKey,
                expected_match_rate: 0.9,
            }],
            lookup_source: Some(LookupSource::Derived {
                code_field: "MUNI".to_string(),
                name_field: "MUNINAME".to_string(),
            }),
            collapse_duplicate_geometry: false,
        },
        attributes: Attributes {
            attributes: vec![Attribute {
                key: "price".to_string(),
                source_table: "sales".to_string(),
                source_column: "amount".to_string(),
                value_type: ValueType::Numeric,
                direction: Direction::Favourable,
                bands: None,
                variance_tested: true,
                variance_tested_on: Some("2026-01-01".to_string()),
            }],
        },
        triggers: Triggers {
            triggers: vec![Trigger {
                key: "new_listing".to_string(),
                source_id: "listings".to_string(),
                published: true,
                wired: true,
                lag_days: Some(7),
                hard_stop: None,
            }],
        },
        work_scope: WorkScope {
            jobs: vec![Job {
                key: "inspect".to_string(),
                label: "Inspect".to_string(),
                ticket_low: 100,
                ticket_high: 500,
                description: None,
            }],
            capacity: Capacity {
                builds_concurrent: 1,
                first_meetings_per_week: 1,
            },
        },
        vocabulary: Vocabulary::from_test_lists(
            vec!["buy".to_string()],
            vec!["spam".to_string()],
            vec!["hello".to_string()],
            vec!["lawsuit".to_string()],
        ),
        zones: Zones {
            assignments: vec![ZoneAssignment {
                capability_key: "score".to_string(),
                zone: Zone::Inside,
                rationale: "baseline".to_string(),
                assigned_on: "2026-01-01".to_string(),
                expires_on: "2026-12-31".to_string(),
                display_caveat: None,
            }],
        },
        boundaries: Boundaries {
            approval_required_by_default: true,
            absence_rules: vec![AbsenceRule {
                item: "ssn".to_string(),
                rule: AbsenceRuleKind::Drop,
            }],
            never_stored: vec!["ssn".to_string()],
            cloud_extraction_allowed_for: vec!["assessor".to_string()],
        },
        coverage: Coverage {
            geographies: vec![CoverageEntry {
                key: "county".to_string(),
                level: CoverageLevel::Full,
                note: "all tracts".to_string(),
            }],
            attributes: vec![CoverageEntry {
                key: "price".to_string(),
                level: CoverageLevel::Full,
                note: "complete".to_string(),
            }],
            publication_lags: vec![PublicationLag {
                source_id: "listings".to_string(),
                lag_days: 7,
                measured: true,
                measured_from: Some("2026-01-01".to_string()),
            }],
        },
        sources: SourcesFile {
            sources: vec![
                live_source("parcels"),
                live_source("sales"),
                live_source("listings"),
            ],
        },
        series: SeriesFile {
            series: vec![SeriesEntry {
                series_id: "HOUST".to_string(),
                label: "Housing starts".to_string(),
                geography: "national".to_string(),
                included: true,
                notes: None,
            }],
        },
        assignments: AssignmentsFile::from_dr_data_file(),
        owner_type: OwnerTypeFile::from_dr_data_file(),
        tiers: TiersFile::test_fixture(),
        search_query_templates: None,
    }
}

fn valid_file_bodies() -> [(&'static str, &'static str); 14] {
    [
        (
            "manifest.toml",
            r#"
schema_version = "1"
config_version = "1"
vertical_id = "test"
vertical_display_name = "Test"
county_fips = "06001"
county_name = "Alameda"
entity_universe = "Parcels"

[[pages]]
key = "status"

[[pages]]
key = "owner_groups"

[[pages]]
key = "assignments"

[[pages]]
key = "market_watch"

[[pages]]
key = "coverage"

[[pages]]
key = "sources"

[[pages]]
key = "how_to_use"
"#,
        ),
        (
            "entity.toml",
            r#"
spine_source_id = "parcels"
county_fips = "06001"
natural_key_field = "apn"
has_stable_key = true

[[layers]]
role = "Primary"
url = "https://example.invalid/primary"
where_clause = "1=1"
vintage_field = "Export_Date"

[layers.field_map]
taxkey = "TAXKEY"
muni_code = "MUNI"
owner_name = "OWNERNAMEFULL"
site_address = "SITEADRESS"
mail_address_1 = "PSTLADRESSLINE1"
mail_address_2 = "PSTLADRESSLINE2"
assessed_value = "CNTASSDVALUE"
improvement_value = "IMPVALUE"
land_value = "LNDVALUE"
prop_class = "PROPCLASS"

[lookup_source]
kind = "Derived"
code_field = "MUNI"
name_field = "MUNINAME"

[[attachments]]
source_id = "sales"
join_field = "apn"
join_method = "ExactKey"
expected_match_rate = 0.9
"#,
        ),
        (
            "attributes.toml",
            r#"
[[attributes]]
key = "price"
source_table = "sales"
source_column = "amount"
value_type = "Numeric"
direction = "Favourable"
variance_tested = true
variance_tested_on = "2026-01-01"
"#,
        ),
        (
            "triggers.toml",
            r#"
[[triggers]]
key = "new_listing"
source_id = "listings"
published = true
wired = true
lag_days = 7
"#,
        ),
        (
            "work_scope.toml",
            r#"
[[jobs]]
key = "inspect"
label = "Inspect"
ticket_low = 100
ticket_high = 500

[capacity]
builds_concurrent = 1
first_meetings_per_week = 1
"#,
        ),
        (
            "vocabulary.toml",
            r#"
[[verticals]]
vertical_key = "test"
classifier_positive = ["buy"]
classifier_negative = ["spam"]
outreach_lead = ["hello"]
outreach_avoid = ["lawsuit"]
"#,
        ),
        (
            "zones.toml",
            r#"
[[assignments]]
capability_key = "score"
zone = "Inside"
rationale = "baseline"
assigned_on = "2026-01-01"
expires_on = "2026-12-31"
"#,
        ),
        (
            "boundaries.toml",
            r#"
approval_required_by_default = true
never_stored = ["ssn"]
cloud_extraction_allowed_for = ["assessor"]

[[absence_rules]]
item = "ssn"
rule = "Drop"
"#,
        ),
        (
            "coverage.toml",
            r#"
[[geographies]]
key = "county"
level = "Full"
note = "all tracts"

[[attributes]]
key = "price"
level = "Full"
note = "complete"

[[publication_lags]]
source_id = "listings"
lag_days = 7
measured = true
measured_from = "2026-01-01"
"#,
        ),
        (
            "sources.toml",
            r#"
[[sources]]
source_id = "parcels"
display_name = "parcels"
family = "PropertyEvent"
kind = "Csv"
status = "Live"
lag_measured = false
url_verified = false
requires_key = false

[[sources]]
source_id = "sales"
display_name = "sales"
family = "PropertyEvent"
kind = "Csv"
status = "Live"
lag_measured = false
url_verified = false
requires_key = false

[[sources]]
source_id = "listings"
display_name = "listings"
family = "PropertyEvent"
kind = "Csv"
status = "Live"
lag_measured = false
url_verified = false
requires_key = false
"#,
        ),
        (
            "series.toml",
            r#"
[[series]]
series_id = "HOUST"
label = "Housing starts"
geography = "national"
included = true
"#,
        ),
        (
            "assignments.toml",
            r#"
state_values = ["WI", "WISCONSIN"]
waukesha_cities = ["Waukesha"]
milwaukee_cities = ["Milwaukee"]
"#,
        ),
        (
            "owner_type.toml",
            r#"
precedence = ["placeholder", "government", "trust", "entity"]
exclude_from_groups_default = ["government", "placeholder"]

[[classes]]
type_key = "placeholder"
phrases = [{ text = "INCOMPLETE TAXROLL INFORMATION" }]

[[classes]]
type_key = "government"
phrases = [
  { text = "CITY OF", anchor = "anywhere" },
  { text = "COUNTY", anchor = "anywhere", not_followed_by = ["ROAD", "RD", "LINE"] },
]

[[classes]]
type_key = "trust"
phrases = [{ text = "TRUST" }]

[[classes]]
type_key = "entity"
phrases = [{ text = "LLC" }]
"#,
        ),
        (
            "tiers.toml",
            r#"
[[tiers]]
tier_key = "1"
label = "Tier 1"
description = "test description for tier 1"
what_they_buy = "test purchase for tier 1"
price_band = "test price band for tier 1"
scoring_checks = [
    "A named decision with money attached",
    "Their own historical data exists somewhere, even in spreadsheets",
    "One person's judgment is doing the work",
    "Data they would not want in a cloud tool",
    "An active trigger: hiring an estimator, succession underway, a recent cost shock, a compliance deadline",
]
scoring_rule = "Five of five is a Tier 1 build. Three or four is worth a discovery meeting. Under three is Tier 3 or a pass."

[[tiers]]
tier_key = "2"
label = "Tier 2"
description = "test description for tier 2"
what_they_buy = "test purchase for tier 2"
price_band = "test price band for tier 2"
scoring_checks = [
    "A named decision with money attached",
    "Their own historical data exists somewhere, even in spreadsheets",
    "One person's judgment is doing the work",
    "Data they would not want in a cloud tool",
    "An active trigger: hiring an estimator, succession underway, a recent cost shock, a compliance deadline",
]
scoring_rule = "Five of five is a Tier 1 build. Three or four is worth a discovery meeting. Under three is Tier 3 or a pass."

[[tiers]]
tier_key = "3"
label = "Tier 3"
description = "test description for tier 3"
what_they_buy = "test purchase for tier 3"
price_band = "test price band for tier 3"
scoring_checks = [
    "A named decision with money attached",
    "Their own historical data exists somewhere, even in spreadsheets",
    "One person's judgment is doing the work",
    "Data they would not want in a cloud tool",
    "An active trigger: hiring an estimator, succession underway, a recent cost shock, a compliance deadline",
]
scoring_rule = "Five of five is a Tier 1 build. Three or four is worth a discovery meeting. Under three is Tier 3 or a pass."
"#,
        ),
    ]
}

fn temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("radar_config_{}_{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_named(dir: &Path, files: &[(&str, &str)]) {
    for (name, body) in files {
        fs::write(dir.join(name), body).expect("write temp config file");
    }
}

fn write_all_valid(dir: &Path) {
    write_named(dir, &valid_file_bodies());
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

fn has_rule(findings: &[crate::Finding], code: &str) -> bool {
    findings.iter().any(|finding| finding.rule_code == code)
}

#[test]
fn missing_file_is_error() {
    let dir = temp_dir();
    let mut files: Vec<(&str, &str)> = valid_file_bodies().into_iter().collect();
    files.retain(|(name, _)| *name != "coverage.toml");
    write_named(&dir, &files);
    let result = load(&dir);
    cleanup(&dir);
    match result {
        Err(ConfigError::FileMissing { path }) => {
            assert!(path.ends_with("coverage.toml"));
        }
        other => panic!("expected FileMissing, got {other:?}"),
    }
}

#[test]
fn unknown_field_is_error() {
    let dir = temp_dir();
    let mut files: Vec<(&str, &str)> = valid_file_bodies().into_iter().collect();
    for (name, body) in files.iter_mut() {
        if *name == "manifest.toml" {
            *body = r#"
schema_version = "1"
config_version = "1"
vertical_id = "test"
vertical_display_name = "Test"
county_fips = "06001"
county_name = "Alameda"
entity_universe = "Parcels"
extra_field = true
"#;
        }
    }
    write_named(&dir, &files);
    let result = load(&dir);
    cleanup(&dir);
    match result {
        Err(ConfigError::UnknownField { field, path }) => {
            assert_eq!(field, "extra_field");
            assert!(path.ends_with("manifest.toml"));
        }
        other => panic!("expected UnknownField, got {other:?}"),
    }
}

#[test]
fn wrong_type_is_parse_failed() {
    let dir = temp_dir();
    let mut files: Vec<(&str, &str)> = valid_file_bodies().into_iter().collect();
    for (name, body) in files.iter_mut() {
        if *name == "entity.toml" {
            *body = r#"
spine_source_id = "parcels"
natural_key_field = "apn"
has_stable_key = "not-a-bool"

[[attachments]]
source_id = "sales"
join_field = "apn"
join_method = "ExactKey"
expected_match_rate = 0.9
"#;
        }
    }
    write_named(&dir, &files);
    let result = load(&dir);
    cleanup(&dir);
    match result {
        Err(ConfigError::ParseFailed { path, message }) => {
            assert!(path.ends_with("entity.toml"));
            assert!(!message.is_empty());
        }
        other => panic!("expected ParseFailed, got {other:?}"),
    }
}

#[test]
fn load_succeeds_when_all_ten_parse() {
    let dir = temp_dir();
    write_all_valid(&dir);
    let result = load(&dir);
    cleanup(&dir);
    let config = result.expect("valid directory should load");
    assert_eq!(config.manifest.vertical_id, "test");
    assert_eq!(config.attributes.attributes.len(), 1);
    assert_eq!(config.series.series.len(), 1);
    assert_eq!(config.series.series[0].series_id, "HOUST");
    assert_eq!(config.tiers.tiers.len(), 3);
    assert_eq!(config.vocabulary.verticals.len(), 1);
    assert_eq!(config.vocabulary.verticals[0].vertical_key, "test");
}

#[test]
fn direction_none_set_rejected() {
    let mut config = valid_config();
    config.attributes.attributes[0].direction = Direction::NoneSet;
    let findings = validate(&config);
    assert!(has_rule(&findings, "V1"));
    assert!(findings
        .iter()
        .any(|finding| finding.rule_code == "V1" && finding.severity == Severity::Error));
}

#[test]
fn band_without_bands_rejected() {
    let mut config = valid_config();
    config.attributes.attributes[0].direction = Direction::Band;
    config.attributes.attributes[0].bands = None;
    let findings = validate(&config);
    assert!(has_rule(&findings, "V2"));
}

#[test]
fn overlapping_bands_rejected() {
    let mut config = valid_config();
    config.attributes.attributes[0].direction = Direction::Band;
    config.attributes.attributes[0].bands = Some(vec![
        Band {
            min: Some(0.0),
            max: Some(100.0),
            label: "low".to_string(),
            favourability: Favourability::Favourable,
        },
        Band {
            min: Some(50.0),
            max: Some(150.0),
            label: "mid".to_string(),
            favourability: Favourability::Neutral,
        },
    ]);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V3"));
}

#[test]
fn capacity_zero_rejected() {
    let mut config = valid_config();
    config.work_scope.capacity.builds_concurrent = 0;
    let findings = validate(&config);
    assert!(has_rule(&findings, "V5"));
}

#[test]
fn below_zone_without_caveat_rejected() {
    let mut config = valid_config();
    config.zones.assignments[0].zone = Zone::Below;
    config.zones.assignments[0].display_caveat = None;
    let findings = validate(&config);
    assert!(has_rule(&findings, "V8"));
}

#[test]
fn term_in_both_list_families_rejected() {
    let mut config = valid_config();
    config.vocabulary.verticals[0].classifier_positive = vec!["alpha".to_string()];
    config.vocabulary.verticals[0].outreach_lead = vec!["alpha".to_string()];
    let findings = validate(&config);
    assert!(has_rule(&findings, "V9"));
}

#[test]
fn warnings_do_not_block_ready() {
    let mut config = valid_config();
    config.attributes.attributes[0].variance_tested = false;
    let state = SessionState::from_config(PathBuf::from("memory"), config);
    match state {
        SessionState::Ready { warnings, .. } => {
            assert!(warnings.iter().any(|finding| finding.rule_code == "V12"));
            assert!(warnings.iter().all(|finding| finding.severity == Severity::Warning));
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn errors_yield_invalid_and_do_not_hold_config() {
    let mut config = valid_config();
    config.attributes.attributes[0].direction = Direction::NoneSet;
    let state = SessionState::from_config(PathBuf::from("memory"), config);
    match state {
        SessionState::Invalid { findings, .. } => {
            assert!(has_rule(&findings, "V1"));
        }
        SessionState::Ready { .. } => panic!("error findings must not yield Ready"),
        SessionState::Unconfigured => panic!("expected Invalid"),
    }
}

#[test]
fn dr_data_icp_config_loads_and_is_ready() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/dr_data_icp");
    let config = load(&dir).expect("load should return Ok");
    let findings = validate(&config);
    let errors: Vec<_> = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "expected zero Error findings, got {errors:?}"
    );
    assert_eq!(config.tiers.tiers[0].label, "Judgment encoding");
    assert!(config
        .vocabulary
        .for_vertical("manufacturer_custom_fabricator")
        .is_some());
    assert!(config
        .vocabulary
        .for_vertical("energy_assessor")
        .is_none());
    let templates = config
        .search_query_templates
        .as_ref()
        .expect("search_query_templates.toml is present for dr_data_icp");
    assert_eq!(templates.categories.len(), 6);
    let state = SessionState::from_dir(&dir).expect("from_dir should succeed");
    match state {
        SessionState::Ready { warnings, .. } => {
            println!("Warning findings:");
            for warning in &warnings {
                println!(
                    "{}: {} [{}]",
                    warning.rule_code, warning.message, warning.location
                );
            }
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn unknown_tier_key_rejected() {
    let mut config = valid_config();
    config.tiers.tiers[0].tier_key = "4".to_string();
    let findings = validate(&config);
    assert!(has_rule(&findings, "V33"));
}

#[test]
fn missing_tier_key_rejected() {
    let mut config = valid_config();
    config.tiers.tiers.pop();
    let findings = validate(&config);
    assert!(has_rule(&findings, "V33"));
}

#[test]
fn unresolved_source_id_rejected() {
    let mut config = valid_config();
    config.triggers.triggers[0].source_id = "missing-source".to_string();
    let findings = validate(&config);
    assert!(has_rule(&findings, "V15"));
}

#[test]
fn non_live_spine_rejected() {
    let mut config = valid_config();
    config.sources.sources[0].status = SourceStatus::Profiled;
    let findings = validate(&config);
    assert!(has_rule(&findings, "V16"));
}

#[test]
fn duplicate_source_id_rejected() {
    let mut config = valid_config();
    let duplicate = config.sources.sources[0].clone();
    config.sources.sources.push(duplicate);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V17"));
}

#[test]
fn spine_coverage_threshold_present_loads_and_is_ready() {
    let dir = temp_dir();
    let mut files: Vec<(&str, &str)> = valid_file_bodies().into_iter().collect();
    for (name, body) in files.iter_mut() {
        if *name == "entity.toml" {
            *body = r#"
spine_source_id = "parcels"
county_fips = "06001"
natural_key_field = "apn"
has_stable_key = true
spine_coverage_fallback_threshold = 0.50

[[layers]]
role = "Primary"
url = "https://example.invalid/primary"
where_clause = "1=1"
vintage_field = "Export_Date"

[layers.field_map]
taxkey = "TAXKEY"
muni_code = "MUNI"
owner_name = "OWNERNAMEFULL"

[[layers]]
role = "Fallback"
url = "https://example.invalid/fallback"
where_clause = "1=1"
vintage_field = "Export_Date"

[layers.field_map]
taxkey = "TAXKEY"
muni_code = "MUNI"
owner_name = "OWNERNAMEFULL"

[lookup_source]
kind = "Derived"
code_field = "MUNI"
name_field = "MUNINAME"

[[attachments]]
source_id = "sales"
join_field = "apn"
join_method = "ExactKey"
expected_match_rate = 0.9
"#;
        }
    }
    write_named(&dir, &files);
    let state = SessionState::from_dir(&dir).expect("load should succeed");
    cleanup(&dir);
    match state {
        SessionState::Ready { config, .. } => {
            assert_eq!(config.entity.spine_coverage_fallback_threshold, Some(0.50));
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn spine_coverage_threshold_omitted_loads_and_is_ready() {
    let dir = temp_dir();
    write_all_valid(&dir);
    let state = SessionState::from_dir(&dir).expect("load should succeed");
    cleanup(&dir);
    match state {
        SessionState::Ready { config, .. } => {
            assert_eq!(config.entity.spine_coverage_fallback_threshold, None);
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn spine_coverage_threshold_zero_fails_v21() {
    let mut config = valid_config();
    config.entity.spine_coverage_fallback_threshold = Some(0.0);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V21"));
    assert!(findings
        .iter()
        .any(|finding| finding.rule_code == "V21" && finding.severity == Severity::Error));
}

#[test]
fn spine_coverage_threshold_one_fails_v21() {
    let mut config = valid_config();
    config.entity.spine_coverage_fallback_threshold = Some(1.0);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V21"));
}

#[test]
fn spine_coverage_threshold_above_one_fails_v21() {
    let mut config = valid_config();
    config.entity.spine_coverage_fallback_threshold = Some(1.5);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V21"));
}

#[test]
fn spine_coverage_threshold_half_passes_v21() {
    let mut config = valid_config();
    config.entity.spine_coverage_fallback_threshold = Some(0.5);
    config.entity.layers.push(fallback_layer());
    let findings = validate(&config);
    assert!(!has_rule(&findings, "V21"));
    let state = SessionState::from_config(PathBuf::from("memory"), config);
    match state {
        SessionState::Ready { .. } => {}
        other => panic!("0.5 must yield Ready, got {other:?}"),
    }
}

#[test]
fn two_primary_layers_fails_v20() {
    let mut config = valid_config();
    config.entity.layers.push(primary_layer());
    let findings = validate(&config);
    assert!(has_rule(&findings, "V20"));
}

#[test]
fn threshold_without_fallback_fails_v24() {
    let mut config = valid_config();
    config.entity.spine_coverage_fallback_threshold = Some(0.5);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V24"));
}

#[test]
fn field_map_missing_owner_name_fails_v23() {
    let mut config = valid_config();
    config.entity.layers[0].field_map.owner_name.clear();
    let findings = validate(&config);
    assert!(has_rule(&findings, "V23"));
}

#[test]
fn county_fips_not_five_digits_fails_v25() {
    let mut config = valid_config();
    config.entity.county_fips = "5513".to_string();
    let findings = validate(&config);
    assert!(has_rule(&findings, "V25"));
}

#[test]
fn missing_lookup_source_fails_v26() {
    let mut config = valid_config();
    config.entity.lookup_source = None;
    let findings = validate(&config);
    assert!(has_rule(&findings, "V26"));
    let state = SessionState::from_config(PathBuf::from("memory"), config);
    match state {
        SessionState::Invalid { findings, .. } => {
            assert!(has_rule(&findings, "V26"));
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn series_missing_id_or_bad_geography_fails_v27() {
    let mut config = valid_config();
    config.series.series[0].series_id.clear();
    let findings = validate(&config);
    assert!(has_rule(&findings, "V27"));

    let mut config = valid_config();
    config.series.series[0].geography = "tract".to_string();
    let findings = validate(&config);
    assert!(has_rule(&findings, "V27"));
}

fn government_county_phrase(config: &mut crate::RadarConfig) -> &mut crate::OwnerTypePhrase {
    let government = config
        .owner_type
        .classes
        .iter_mut()
        .find(|class| class.type_key == "government")
        .expect("government class");
    government
        .phrases
        .iter_mut()
        .find(|phrase| phrase.text == "COUNTY")
        .expect("COUNTY phrase")
}

#[test]
fn empty_not_followed_by_list_fails_v31() {
    let mut config = valid_config();
    government_county_phrase(&mut config).not_followed_by = Some(Vec::new());
    let findings = validate(&config);
    assert!(has_rule(&findings, "V31"));
    assert!(findings.iter().any(|finding| {
        finding.rule_code == "V31"
            && finding.severity == Severity::Error
            && finding.message.contains("empty list")
    }));
}

#[test]
fn empty_not_followed_by_entry_fails_v31() {
    let mut config = valid_config();
    government_county_phrase(&mut config).not_followed_by =
        Some(vec!["ROAD".to_string(), "   ".to_string()]);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V31"));
    assert!(findings.iter().any(|finding| {
        finding.rule_code == "V31"
            && finding.message.contains("must not be empty")
    }));
}

#[test]
fn phrase_whose_every_token_is_excluded_fails_v31() {
    let mut config = valid_config();
    let phrase = government_county_phrase(&mut config);
    phrase.text = "COUNTY ROAD".to_string();
    phrase.not_followed_by = Some(vec!["COUNTY".to_string(), "ROAD".to_string()]);
    let findings = validate(&config);
    assert!(has_rule(&findings, "V31"));
    assert!(findings.iter().any(|finding| {
        finding.rule_code == "V31"
            && finding.message.contains("every token is excluded")
    }));
}

fn page(key: &str) -> PageEntry {
    PageEntry {
        key: key.to_string(),
        label: None,
    }
}

fn nav_keys(config: &RadarConfig) -> Vec<String> {
    nav_pages(&config.manifest)
        .into_iter()
        .map(|page| page.key)
        .collect()
}

#[test]
fn four_pages_render_four_nav_entries_in_listed_order() {
    let mut config = valid_config();
    config.manifest.pages = vec![
        page("status"),
        page("owner_groups"),
        page("assignments"),
        page("how_to_use"),
    ];
    let findings = validate(&config);
    assert!(!has_rule(&findings, "V32"));
    let nav = nav_pages(&config.manifest);
    assert_eq!(nav.len(), 4);
    assert_eq!(
        nav.iter().map(|page| page.key.as_str()).collect::<Vec<_>>(),
        vec!["status", "owner_groups", "assignments", "how_to_use"]
    );
    assert_eq!(
        nav.iter().map(|page| page.label.as_str()).collect::<Vec<_>>(),
        vec!["Status", "Owner groups", "Assignments", "How to use this"]
    );
}

#[test]
fn omitting_status_fails_v32() {
    let mut config = valid_config();
    config.manifest.pages = vec![page("owner_groups"), page("how_to_use")];
    let findings = validate(&config);
    assert!(has_rule(&findings, "V32"));
    assert!(findings.iter().any(|finding| {
        finding.rule_code == "V32"
            && finding.severity == Severity::Error
            && finding.message.contains("status")
    }));
}

#[test]
fn unknown_page_key_fails_v32_and_is_named() {
    let mut config = valid_config();
    config.manifest.pages.push(page("leads_board"));
    let findings = validate(&config);
    assert!(has_rule(&findings, "V32"));
    assert!(findings.iter().any(|finding| {
        finding.rule_code == "V32"
            && finding.severity == Severity::Error
            && finding.message.contains("leads_board")
    }));
}

#[test]
fn duplicate_page_key_fails_v32() {
    let mut config = valid_config();
    config.manifest.pages.push(page("status"));
    let findings = validate(&config);
    assert!(has_rule(&findings, "V32"));
    assert!(findings.iter().any(|finding| {
        finding.rule_code == "V32"
            && finding.severity == Severity::Error
            && finding.message.contains("duplicate")
            && finding.message.contains("status")
    }));
}

#[test]
fn milwaukee_parcels_hides_market_watch() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/milwaukee_parcels");
    let config = load(&dir).expect("load milwaukee_parcels");
    let findings = validate(&config);
    let errors: Vec<_> = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "expected zero Error findings, got {errors:?}");
    let keys = nav_keys(&config);
    assert_eq!(
        keys,
        vec![
            "status".to_string(),
            "owner_groups".to_string(),
            "assignments".to_string(),
            "coverage".to_string(),
            "sources".to_string(),
            "how_to_use".to_string()
        ]
    );
    assert!(!keys.iter().any(|key| key == "market_watch"));
}

#[test]
fn changing_page_order_changes_nav_order_with_no_code_edit() {
    let mut config = valid_config();
    config.manifest.pages = vec![
        page("how_to_use"),
        page("status"),
        page("assignments"),
        page("owner_groups"),
        page("market_watch"),
    ];
    assert!(!has_rule(&validate(&config), "V32"));
    assert_eq!(
        nav_keys(&config),
        vec![
            "how_to_use".to_string(),
            "status".to_string(),
            "assignments".to_string(),
            "owner_groups".to_string(),
            "market_watch".to_string()
        ]
    );
    config.manifest.pages = vec![
        page("status"),
        page("market_watch"),
        page("owner_groups"),
        page("assignments"),
        page("how_to_use"),
    ];
    assert_eq!(
        nav_keys(&config),
        vec![
            "status".to_string(),
            "market_watch".to_string(),
            "owner_groups".to_string(),
            "assignments".to_string(),
            "how_to_use".to_string()
        ]
    );
}

