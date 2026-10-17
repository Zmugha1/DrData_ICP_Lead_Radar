use std::collections::HashSet;

use crate::attributes::{Attribute, Band, Direction};
use crate::manifest::{known_page_key, REQUIRED_PAGE_KEYS};
use crate::owner_type::tokenize;
use crate::sources::SourceStatus;
use crate::zones::Zone;
use crate::RadarConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule_code: String,
    pub severity: Severity,
    pub message: String,
    pub location: String,
}

pub fn validate(config: &RadarConfig) -> Vec<Finding> {
    let mut findings = Vec::new();

    for (index, attribute) in config.attributes.attributes.iter().enumerate() {
        let location = format!("attributes[{index}] (key={})", attribute.key);
        check_v1(attribute, &location, &mut findings);
        check_v2(attribute, &location, &mut findings);
        check_v3(attribute, &location, &mut findings);
        if !attribute.variance_tested {
            findings.push(finding(
                "V12",
                Severity::Warning,
                "attribute has not been variance tested",
                &location,
            ));
        }
    }

    for (index, attachment) in config.entity.attachments.iter().enumerate() {
        let location = format!("entity.attachments[{index}] (source_id={})", attachment.source_id);
        if !(0.0..=1.0).contains(&attachment.expected_match_rate) {
            findings.push(finding(
                "V4",
                Severity::Error,
                "expected_match_rate must be between 0.0 and 1.0",
                &location,
            ));
        }
    }

    if config.work_scope.capacity.builds_concurrent < 1 {
        findings.push(finding(
            "V5",
            Severity::Error,
            "capacity.builds_concurrent must be at least 1",
            "work_scope.capacity.builds_concurrent",
        ));
    }

    if config.work_scope.capacity.first_meetings_per_week < 1 {
        findings.push(finding(
            "V6",
            Severity::Error,
            "capacity.first_meetings_per_week must be at least 1",
            "work_scope.capacity.first_meetings_per_week",
        ));
    }

    for (index, assignment) in config.zones.assignments.iter().enumerate() {
        let location = format!(
            "zones.assignments[{index}] (capability_key={})",
            assignment.capability_key
        );
        if !is_iso_date(&assignment.expires_on) {
            findings.push(finding(
                "V7",
                Severity::Error,
                "expires_on must parse as a date (YYYY-MM-DD)",
                &location,
            ));
        }
        if assignment.zone == Zone::Below {
            let missing = match &assignment.display_caveat {
                None => true,
                Some(text) => text.is_empty(),
            };
            if missing {
                findings.push(finding(
                    "V8",
                    Severity::Error,
                    "zone Below requires a non-empty display_caveat",
                    &location,
                ));
            }
        }
    }

    let mut seen_vocab_keys = HashSet::new();
    for (index, entry) in config.vocabulary.verticals.iter().enumerate() {
        let location = format!(
            "vocabulary.verticals[{index}] (vertical_key={})",
            entry.vertical_key
        );
        if entry.vertical_key.trim().is_empty() {
            findings.push(finding(
                "V9",
                Severity::Error,
                "vertical_key must not be empty",
                &location,
            ));
        }
        if !seen_vocab_keys.insert(entry.vertical_key.clone()) {
            findings.push(finding(
                "V9",
                Severity::Error,
                format!("duplicate vertical_key {}", entry.vertical_key),
                &location,
            ));
        }
        let classifier: HashSet<&str> = entry
            .classifier_positive
            .iter()
            .chain(entry.classifier_negative.iter())
            .map(String::as_str)
            .collect();
        for (family, terms) in [
            ("outreach_lead", &entry.outreach_lead),
            ("outreach_avoid", &entry.outreach_avoid),
        ] {
            for term in terms {
                if classifier.contains(term.as_str()) {
                    findings.push(finding(
                        "V9",
                        Severity::Error,
                        format!("term {term} appears in both a classifier list and {family}"),
                        format!("{location}.{family}"),
                    ));
                }
            }
        }
    }

    let unwired_sources: HashSet<&str> = config
        .triggers
        .triggers
        .iter()
        .filter(|trigger| !trigger.wired)
        .map(|trigger| trigger.source_id.as_str())
        .collect();
    for (index, attribute) in config.attributes.attributes.iter().enumerate() {
        if unwired_sources.contains(attribute.source_table.as_str()) {
            let location = format!("attributes[{index}] (key={})", attribute.key);
            findings.push(finding(
                "V10",
                Severity::Error,
                format!(
                    "attribute source_table {} belongs to a trigger with wired = false",
                    attribute.source_table
                ),
                &location,
            ));
        }
    }

    for (index, trigger) in config.triggers.triggers.iter().enumerate() {
        let location = format!("triggers[{index}] (key={})", trigger.key);
        if !trigger.published && trigger.wired {
            findings.push(finding(
                "V11",
                Severity::Error,
                "a trigger with published = false must also have wired = false",
                &location,
            ));
        }
        if !trigger.wired {
            findings.push(finding(
                "V13",
                Severity::Warning,
                "trigger is not wired",
                &location,
            ));
        }
    }

    let key_present = config.entity.natural_key_field.is_some();
    if config.entity.has_stable_key != key_present {
        findings.push(finding(
            "V14",
            Severity::Error,
            "has_stable_key must be false when natural_key_field is absent, and true only when natural_key_field is present",
            "entity.has_stable_key",
        ));
    }

    let mut seen_ids = HashSet::new();
    let mut duplicates = HashSet::new();
    for source in &config.sources.sources {
        if !seen_ids.insert(source.source_id.as_str()) {
            duplicates.insert(source.source_id.as_str());
        }
    }
    for source_id in &duplicates {
        findings.push(finding(
            "V17",
            Severity::Error,
            format!("duplicate source_id {source_id}"),
            format!("sources (source_id={source_id})"),
        ));
    }

    let registry: HashSet<&str> = config
        .sources
        .sources
        .iter()
        .map(|source| source.source_id.as_str())
        .collect();

    let mut referenced: Vec<(&str, String)> = Vec::new();
    referenced.push((
        config.entity.spine_source_id.as_str(),
        "entity.spine_source_id".to_string(),
    ));
    for (index, attachment) in config.entity.attachments.iter().enumerate() {
        referenced.push((
            attachment.source_id.as_str(),
            format!("entity.attachments[{index}] (source_id={})", attachment.source_id),
        ));
    }
    for (index, trigger) in config.triggers.triggers.iter().enumerate() {
        referenced.push((
            trigger.source_id.as_str(),
            format!("triggers[{index}] (key={})", trigger.key),
        ));
    }

    for (source_id, location) in &referenced {
        if !registry.contains(source_id) {
            findings.push(finding(
                "V15",
                Severity::Error,
                format!("source_id {source_id} is not registered in sources.toml"),
                location.clone(),
            ));
        }
    }

    // Attributes cannot be checked this way because Attribute carries
    // source_table and source_column rather than a source_id. V10 covers
    // the attribute-to-trigger direction.
    if let Some(spine) = config
        .sources
        .sources
        .iter()
        .find(|source| source.source_id == config.entity.spine_source_id)
    {
        if spine.status != SourceStatus::Live {
            findings.push(finding(
                "V16",
                Severity::Error,
                format!(
                    "spine_source_id {} references a source whose status is not Live",
                    config.entity.spine_source_id
                ),
                "entity.spine_source_id",
            ));
        }
    }
    for (index, attachment) in config.entity.attachments.iter().enumerate() {
        if let Some(source) = config
            .sources
            .sources
            .iter()
            .find(|entry| entry.source_id == attachment.source_id)
        {
            if source.status != SourceStatus::Live {
                findings.push(finding(
                    "V16",
                    Severity::Error,
                    format!(
                        "attachment source_id {} references a source whose status is not Live",
                        attachment.source_id
                    ),
                    format!(
                        "entity.attachments[{index}] (source_id={})",
                        attachment.source_id
                    ),
                ));
            }
        }
    }

    let mut referenced_anywhere: HashSet<&str> = referenced.iter().map(|(id, _)| *id).collect();
    for lag in &config.coverage.publication_lags {
        referenced_anywhere.insert(lag.source_id.as_str());
    }
    for source in &config.sources.sources {
        if source.status == SourceStatus::Refused && referenced_anywhere.contains(source.source_id.as_str())
        {
            findings.push(finding(
                "V18",
                Severity::Warning,
                format!("source {} has status Refused and is referenced", source.source_id),
                format!("sources (source_id={})", source.source_id),
            ));
        }
    }

    for source in &config.sources.sources {
        if source.publication_lag_days.is_some() && !source.lag_measured {
            findings.push(finding(
                "V19",
                Severity::Warning,
                format!(
                    "source {} has publication_lag_days set and lag_measured false",
                    source.source_id
                ),
                format!("sources (source_id={})", source.source_id),
            ));
        }
    }

    if let Some(threshold) = config.entity.spine_coverage_fallback_threshold {
        if !(threshold > 0.0 && threshold < 1.0) {
            findings.push(finding(
                "V21",
                Severity::Error,
                "spine_coverage_fallback_threshold must be greater than 0.0 and less than 1.0",
                "entity.spine_coverage_fallback_threshold",
            ));
        }
    }

    let primary_count = config
        .entity
        .layers
        .iter()
        .filter(|layer| layer.role == crate::entity::LayerRole::Primary)
        .count();
    if primary_count != 1 {
        findings.push(finding(
            "V20",
            Severity::Error,
            "exactly one layer must have role Primary",
            "entity.layers",
        ));
    }

    let fallback_count = config
        .entity
        .layers
        .iter()
        .filter(|layer| layer.role == crate::entity::LayerRole::Fallback)
        .count();
    if fallback_count > 1 {
        findings.push(finding(
            "V22",
            Severity::Error,
            "at most one layer may have role Fallback",
            "entity.layers",
        ));
    }

    for (index, layer) in config.entity.layers.iter().enumerate() {
        let location = format!("entity.layers[{index}]");
        if layer.field_map.taxkey.trim().is_empty()
            || layer.field_map.muni_code.trim().is_empty()
            || layer.field_map.owner_name.trim().is_empty()
        {
            findings.push(finding(
                "V23",
                Severity::Error,
                "field_map must carry taxkey, muni_code and owner_name",
                location,
            ));
        }
    }

    let has_fallback = fallback_count > 0;
    let has_threshold = config.entity.spine_coverage_fallback_threshold.is_some();
    if has_threshold != has_fallback {
        findings.push(finding(
            "V24",
            Severity::Error,
            "spine_coverage_fallback_threshold is present only when a Fallback layer exists, and absent when none does",
            "entity.spine_coverage_fallback_threshold",
        ));
    }

    if config.entity.county_fips.len() != 5
        || !config.entity.county_fips.chars().all(|c| c.is_ascii_digit())
    {
        findings.push(finding(
            "V25",
            Severity::Error,
            "county_fips must be five digits",
            "entity.county_fips",
        ));
    }

    match &config.entity.lookup_source {
        None => {
            findings.push(finding(
                "V26",
                Severity::Error,
                "lookup_source is required; a county with neither a Layer nor a Derived lookup fails validation rather than failing E1/E2",
                "entity.lookup_source",
            ));
        }
        Some(crate::entity::LookupSource::Layer {
            url,
            code_field,
            name_field,
            ..
        }) => {
            if url.trim().is_empty() || code_field.trim().is_empty() || name_field.trim().is_empty()
            {
                findings.push(finding(
                    "V26",
                    Severity::Error,
                    "lookup_source Layer requires url, code_field and name_field",
                    "entity.lookup_source",
                ));
            }
        }
        Some(crate::entity::LookupSource::Derived {
            code_field,
            name_field,
        }) => {
            if code_field.trim().is_empty() || name_field.trim().is_empty() {
                findings.push(finding(
                    "V26",
                    Severity::Error,
                    "lookup_source Derived requires code_field and name_field",
                    "entity.lookup_source",
                ));
            }
        }
    }

    const GEOGRAPHIES: [&str; 3] = ["national", "state", "county"];
    for (index, entry) in config.series.series.iter().enumerate() {
        let location = format!("series[{index}] (series_id={})", entry.series_id);
        if entry.series_id.trim().is_empty() || entry.geography.trim().is_empty() {
            findings.push(finding(
                "V27",
                Severity::Error,
                "every series entry must have a series_id and a geography",
                &location,
            ));
        } else if !GEOGRAPHIES.contains(&entry.geography.as_str()) {
            findings.push(finding(
                "V27",
                Severity::Error,
                "geography must be one of national, state, county",
                &location,
            ));
        }
    }

    if config.assignments.state_values.is_empty() {
        findings.push(finding(
            "V28",
            Severity::Error,
            "assignments.state_values must not be empty",
            "assignments.state_values",
        ));
    }
    if config.assignments.waukesha_cities.is_empty() || config.assignments.milwaukee_cities.is_empty()
    {
        findings.push(finding(
            "V28",
            Severity::Error,
            "assignments city lists must not be empty",
            "assignments",
        ));
    }

    if config.owner_type.precedence.is_empty() || config.owner_type.classes.is_empty() {
        findings.push(finding(
            "V29",
            Severity::Error,
            "owner_type.precedence and owner_type.classes must not be empty",
            "owner_type",
        ));
    }
    for (index, type_key) in config.owner_type.precedence.iter().enumerate() {
        if !config
            .owner_type
            .classes
            .iter()
            .any(|class| &class.type_key == type_key)
        {
            findings.push(finding(
                "V29",
                Severity::Error,
                "precedence entry must name a class type_key",
                &format!("owner_type.precedence[{index}]"),
            ));
        }
    }
    for (index, class) in config.owner_type.classes.iter().enumerate() {
        if class.type_key.trim().is_empty() || class.phrases.is_empty() {
            findings.push(finding(
                "V29",
                Severity::Error,
                "each owner-type class must have a type_key and a non-empty phrases list",
                &format!("owner_type.classes[{index}]"),
            ));
        }
        // Phrase `anchor` is first | final | anywhere (default anywhere).
        // Unknown values fail TOML parse via PhraseAnchor. Empty text is V30.
        // Phrase `not_followed_by` is an optional list of single tokens.
        // Empty list, empty entry, or a phrase whose every token is in
        // that list is V31 (would otherwise silently match nothing).
        for (phrase_index, phrase) in class.phrases.iter().enumerate() {
            let location = format!("owner_type.classes[{index}].phrases[{phrase_index}]");
            if phrase.text.trim().is_empty() {
                findings.push(finding(
                    "V30",
                    Severity::Error,
                    "owner-type phrase text must not be empty",
                    &location,
                ));
            }
            if let Some(nfb) = phrase.not_followed_by.as_ref() {
                if nfb.is_empty() {
                    findings.push(finding(
                        "V31",
                        Severity::Error,
                        "not_followed_by must not be an empty list",
                        &location,
                    ));
                }
                let mut excluded: Vec<String> = Vec::new();
                for (entry_index, entry) in nfb.iter().enumerate() {
                    let tokens = tokenize(entry);
                    if tokens.is_empty() {
                        findings.push(finding(
                            "V31",
                            Severity::Error,
                            "not_followed_by entry must not be empty",
                            &format!("{location}.not_followed_by[{entry_index}]"),
                        ));
                    } else if tokens.len() != 1 {
                        findings.push(finding(
                            "V31",
                            Severity::Error,
                            "not_followed_by entry must be a single token",
                            &format!("{location}.not_followed_by[{entry_index}]"),
                        ));
                    } else {
                        excluded.push(tokens[0].clone());
                    }
                }
                let phrase_tokens = tokenize(&phrase.text);
                if !phrase_tokens.is_empty()
                    && phrase_tokens
                        .iter()
                        .all(|token| excluded.iter().any(|excluded| excluded == token))
                {
                    findings.push(finding(
                        "V31",
                        Severity::Error,
                        "phrase would never match: every token is excluded by not_followed_by",
                        &location,
                    ));
                }
            }
        }
    }
    for (index, excluded) in config
        .owner_type
        .exclude_from_groups_default
        .iter()
        .enumerate()
    {
        if !config
            .owner_type
            .classes
            .iter()
            .any(|class| &class.type_key == excluded)
            && excluded != "individual"
        {
            findings.push(finding(
                "V29",
                Severity::Error,
                "exclude_from_groups_default must name a classified type",
                &format!("owner_type.exclude_from_groups_default[{index}]"),
            ));
        }
    }

    check_v32_pages(config, &mut findings);
    check_v33_tiers(config, &mut findings);
    check_v34_search_templates(config, &mut findings);

    findings
}

fn check_v33_tiers(config: &RadarConfig, findings: &mut Vec<Finding>) {
    const REQUIRED_KEYS: [&str; 3] = ["1", "2", "3"];
    let mut seen = HashSet::new();
    for (index, tier) in config.tiers.tiers.iter().enumerate() {
        let location = format!("tiers[{index}] (tier_key={})", tier.tier_key);
        if !REQUIRED_KEYS.contains(&tier.tier_key.as_str()) {
            findings.push(finding(
                "V33",
                Severity::Error,
                format!(
                    "tier_key {} is not a real key; only 1, 2, or 3 are allowed",
                    tier.tier_key
                ),
                &location,
            ));
        }
        if !seen.insert(tier.tier_key.clone()) {
            findings.push(finding(
                "V33",
                Severity::Error,
                format!("duplicate tier_key {}", tier.tier_key),
                &location,
            ));
        }
        if tier.label.trim().is_empty()
            || tier.description.trim().is_empty()
            || tier.what_they_buy.trim().is_empty()
            || tier.price_band.trim().is_empty()
            || tier.scoring_rule.trim().is_empty()
        {
            findings.push(finding(
                "V33",
                Severity::Error,
                "each tier must have a non-empty label, description, what_they_buy, price_band, and scoring_rule",
                &location,
            ));
        }
        if tier.scoring_checks.len() != 5
            || tier.scoring_checks.iter().any(|check| check.trim().is_empty())
        {
            findings.push(finding(
                "V33",
                Severity::Error,
                "each tier must carry the five-check scoring list",
                &location,
            ));
        }
    }
    if let Some(first) = config.tiers.tiers.first() {
        for (index, tier) in config.tiers.tiers.iter().enumerate().skip(1) {
            if tier.scoring_checks != first.scoring_checks || tier.scoring_rule != first.scoring_rule
            {
                findings.push(finding(
                    "V33",
                    Severity::Error,
                    "scoring_checks and scoring_rule must be identical across all tiers",
                    format!("tiers[{index}] (tier_key={})", tier.tier_key),
                ));
            }
        }
    }
    for required in REQUIRED_KEYS {
        if !config.tiers.contains_key(required) {
            findings.push(finding(
                "V33",
                Severity::Error,
                format!("tiers.toml must define tier_key {required}"),
                "tiers",
            ));
        }
    }
}

fn check_v34_search_templates(config: &RadarConfig, findings: &mut Vec<Finding>) {
    let Some(templates) = config.search_query_templates.as_ref() else {
        return;
    };
    for (index, category) in templates.categories.iter().enumerate() {
        let location = format!("search_query_templates.categories[{index}] (key={})", category.key);
        if category.key.trim().is_empty() {
            findings.push(finding(
                "V34",
                Severity::Error,
                "search template category key must not be empty",
                &location,
            ));
        }
        if category.vertical_keys.is_empty()
            || category.vertical_keys.iter().any(|key| key.trim().is_empty())
        {
            findings.push(finding(
                "V34",
                Severity::Error,
                "each search template category must name at least one vertical_key",
                &location,
            ));
        }
        if category.queries.is_empty() || category.queries.iter().any(|query| query.trim().is_empty())
        {
            findings.push(finding(
                "V34",
                Severity::Error,
                "each search template category must have a non-empty queries list",
                &location,
            ));
        }
    }
}

fn check_v32_pages(config: &RadarConfig, findings: &mut Vec<Finding>) {
    let mut seen = HashSet::new();
    for (index, page) in config.manifest.pages.iter().enumerate() {
        let location = format!("manifest.pages[{index}]");
        if !known_page_key(&page.key) {
            findings.push(finding(
                "V32",
                Severity::Error,
                format!("unknown page key {}", page.key),
                &location,
            ));
        }
        if !seen.insert(page.key.clone()) {
            findings.push(finding(
                "V32",
                Severity::Error,
                format!("duplicate page key {}", page.key),
                &location,
            ));
        }
    }
    for required in REQUIRED_PAGE_KEYS {
        if !config.manifest.pages.iter().any(|page| page.key == *required) {
            let reason = if *required == "status" {
                "pages must include status; the config path is set there"
            } else {
                "pages must include how_to_use; the caveats live there"
            };
            findings.push(finding(
                "V32",
                Severity::Error,
                reason,
                "manifest.pages",
            ));
        }
    }
}

fn check_v1(attribute: &Attribute, location: &str, findings: &mut Vec<Finding>) {
    if attribute.direction == Direction::NoneSet {
        findings.push(finding(
            "V1",
            Severity::Error,
            "attribute direction must not be NoneSet",
            location,
        ));
    }
}

fn check_v2(attribute: &Attribute, location: &str, findings: &mut Vec<Finding>) {
    let band_count = attribute
        .bands
        .as_ref()
        .map(|bands| bands.len())
        .unwrap_or(0);
    if attribute.direction == Direction::Band {
        if band_count == 0 {
            findings.push(finding(
                "V2",
                Severity::Error,
                "attribute with direction Band must have a non-empty bands list",
                location,
            ));
        }
    } else if band_count > 0 {
        findings.push(finding(
            "V2",
            Severity::Error,
            "attribute without direction Band must have no bands",
            location,
        ));
    }
}

fn check_v3(attribute: &Attribute, location: &str, findings: &mut Vec<Finding>) {
    let Some(bands) = attribute.bands.as_ref() else {
        return;
    };
    if bands.len() < 2 {
        if bands.len() == 1 {
            if band_inverted(&bands[0]) {
                findings.push(finding(
                    "V3",
                    Severity::Error,
                    "bands overlap or are not ordered",
                    location,
                ));
            }
        }
        return;
    }
    if bands_overlap_or_unordered(bands) {
        findings.push(finding(
            "V3",
            Severity::Error,
            "bands overlap or are not ordered",
            location,
        ));
    }
}

fn band_inverted(band: &Band) -> bool {
    match (band.min, band.max) {
        (Some(min), Some(max)) => max < min,
        _ => false,
    }
}

fn bands_overlap_or_unordered(bands: &[Band]) -> bool {
    for window in bands.windows(2) {
        if band_inverted(&window[0]) || band_inverted(&window[1]) {
            return true;
        }
        let prev_min = window[0].min.unwrap_or(f64::NEG_INFINITY);
        let prev_max = window[0].max.unwrap_or(f64::INFINITY);
        let next_min = window[1].min.unwrap_or(f64::NEG_INFINITY);
        if next_min < prev_min {
            return true;
        }
        if next_min < prev_max {
            return true;
        }
    }
    band_inverted(bands.last().expect("bands is non-empty"))
}

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let Ok(year) = value[0..4].parse::<i32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    if month < 1 || month > 12 || day < 1 {
        return false;
    }
    day <= days_in_month(year, month)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn finding(
    rule_code: &str,
    severity: Severity,
    message: impl Into<String>,
    location: impl Into<String>,
) -> Finding {
    Finding {
        rule_code: rule_code.to_string(),
        severity,
        message: message.into(),
        location: location.into(),
    }
}
