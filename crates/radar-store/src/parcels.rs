use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{params, Connection, Transaction};

use crate::error::StoreError;
use radar_config::{Entity, FieldMap, Layer, LookupSource};

// These values were found when the residential filter was removed on
// 2 September 2026. They are county placeholders rather than parcel
// keys: gaps between parcels, overlapping survey areas, pending
// assignments, common areas and city-held land. They are excluded
// by name rather than by a DISTINCT clause so the count stays visible.
//
// On 2 September 2026 this rule failed the run on a single TL row.
// Blocking 165,266 valid rows on one unrecognised value was the wrong
// trade. The rule now skips and reports, because a value with no
// digit cannot be a parcel key whatever it stands for. Silently
// dropping is the failure this design prevents: the count and the
// value both stay visible on the run row.
const PLACEHOLDER_TAXKEYS: &[&str] = &["GAP", "OVERLAP", "WAKCPENDING", "COMMON", "CITY"];
pub const PARCEL_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const PAGE_RETRIES: u32 = 2;
const VALUE_SOURCE_LIVE: &str = "live";
const VALUE_SOURCE_HISTORIC: &str = "historic_year_end";
const EXACT_PROPCLASS_1_COUNT: i64 = 78417;
const COMPARE_QUERY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
pub struct RawParcel {
    pub taxkey: String,
    pub taxkey_required_normalisation: bool,
    pub object_id: Option<i64>,
    pub muni_code: Option<String>,
    pub lookup_muni_code: Option<String>,
    pub lookup_muni_name: Option<String>,
    pub owner_name: Option<String>,
    pub site_address: Option<String>,
    pub mail_address_1: Option<String>,
    pub mail_address_2: Option<String>,
    pub assessed_value: Option<f64>,
    pub improvement_value: Option<f64>,
    pub land_value: Option<f64>,
    pub owner_type: Option<String>,
    pub mail_matches_site: Option<i64>,
    pub prop_class: Option<String>,
    pub rec_source: Option<String>,
    pub value_as_of: Option<String>,
    pub export_date: Option<String>,
    pub value_source: Option<String>,
    pub source_layer_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParcelLoadResult {
    pub load_run_id: i64,
    pub status: String,
    pub records_requested: i64,
    pub records_written: i64,
    pub records_skipped: i64,
    pub taxkeys_normalised: i64,
    pub taxkeys_normalised_pass1: i64,
    pub taxkeys_normalised_pass2: i64,
    pub page_size: i64,
    pub historic_page_size: i64,
    pub pages_fetched: i64,
    pub pass1_count: i64,
    pub pass2_count: i64,
    pub fallback_codes: Vec<String>,
    pub uncovered_codes: Vec<String>,
    pub live_export_dates: Vec<String>,
    pub historic_export_dates: Vec<String>,
    pub cross_pass_duplicate_taxkeys: i64,
    pub pass2_excluded: i64,
    pub measured_lag_days: Option<i64>,
    pub live_lag_days: Option<i64>,
    pub historic_lag_days: Option<i64>,
}

pub trait ParcelFetch {
    fn max_record_count(&self, layer_url: &str) -> Result<i64, StoreError>;
    fn return_count_only(&self, layer_url: &str, where_clause: &str) -> Result<i64, StoreError>;
    fn distinct_prop_class(
        &self,
        layer_url: &str,
        where_clause: &str,
        prop_class_field: &str,
        taxkey_field: &str,
    ) -> Result<BTreeSet<Option<String>>, StoreError>;
    fn query_page(
        &self,
        layer_url: &str,
        where_clause: &str,
        offset: i64,
        page_size: i64,
        field_map: &FieldMap,
        vintage_field: Option<&str>,
        lookup_fields: Option<(&str, &str)>,
    ) -> Result<Vec<RawParcel>, StoreError>;
}

pub struct LiveParcelSource {
    pub user_agent: String,
    pub min_interval: Duration,
    last_request: Mutex<Option<Instant>>,
}

impl LiveParcelSource {
    pub fn new() -> Self {
        Self {
            user_agent: PARCEL_USER_AGENT.to_string(),
            min_interval: Duration::from_millis(500),
            last_request: Mutex::new(None),
        }
    }

    fn pace(&self) {
        if let Ok(guard) = self.last_request.lock() {
            if let Some(last) = *guard {
                let elapsed = last.elapsed();
                if elapsed < self.min_interval {
                    drop(guard);
                    thread::sleep(self.min_interval.saturating_sub(elapsed));
                }
            }
        }
        if let Ok(mut guard) = self.last_request.lock() {
            *guard = Some(Instant::now());
        }
    }

    fn get_json(&self, url: &str) -> Result<serde_json::Value, StoreError> {
        self.pace();
        let resp = ureq::get(url)
            .set("User-Agent", &self.user_agent)
            .call()
            .map_err(|err| StoreError::ParcelFetch(format!("{err} url={url}")))?;
        let status = resp.status();
        if status != 200 {
            return Err(StoreError::ParcelFetch(format!(
                "http {status} url={url}"
            )));
        }
        let text = resp
            .into_string()
            .map_err(|err| StoreError::ParcelFetch(format!("{err} url={url}")))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|err| StoreError::ParcelFetch(format!("json: {err} url={url}")))?;
        if let Some(message) = value.get("error").and_then(|e| e.get("message")) {
            return Err(StoreError::ParcelFetch(format!(
                "arcgis error {message} url={url}"
            )));
        }
        Ok(value)
    }

    fn get_json_retry(&self, url: &str) -> Result<serde_json::Value, StoreError> {
        let mut last = None;
        for attempt in 0..=PAGE_RETRIES {
            match self.get_json(url) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    eprintln!("request failed attempt {} url={} err={}", attempt + 1, url, err);
                    last = Some(err);
                }
            }
        }
        Err(last.unwrap_or_else(|| StoreError::ParcelFetch("request failed".to_string())))
    }

    pub fn group_counts(
        &self,
        layer_url: &str,
        field: &str,
        where_clause: &str,
        count_field: &str,
    ) -> Result<Vec<(Option<String>, i64)>, StoreError> {
        let stats = format!(
            r#"[{{"statisticType":"count","onStatisticField":"{count_field}","outStatisticFieldName":"cnt"}}]"#
        );
        let url = format!(
            "{layer_url}/query?where={}&groupByFieldsForStatistics={}&outStatistics={}&orderByFields={}&f=json",
            encode_query_component(where_clause),
            encode_query_component(field),
            encode_query_component(&stats),
            encode_query_component(field),
        );
        let value = self.get_json_retry(&url)?;
        let features = value
            .get("features")
            .and_then(|v| v.as_array())
            .ok_or_else(|| StoreError::ParcelFetch("groupBy features array absent".to_string()))?;
        let mut rows = Vec::with_capacity(features.len());
        for feature in features {
            let attrs = feature.get("attributes").ok_or_else(|| {
                StoreError::ParcelFetch("groupBy feature attributes absent".to_string())
            })?;
            let key = json_attr_string(attrs, field);
            let cnt = attrs
                .get("cnt")
                .or_else(|| attrs.get("CNT"))
                .and_then(|v| v.as_i64())
                .ok_or_else(|| StoreError::ParcelFetch("groupBy cnt absent".to_string()))?;
            rows.push((key, cnt));
        }
        Ok(rows)
    }

    pub fn query_all_attributes(
        &self,
        layer_url: &str,
        where_clause: &str,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let url = format!(
            "{layer_url}/query?where={}&outFields=*&returnGeometry=false&resultRecordCount=1&f=json",
            encode_query_component(where_clause)
        );
        let value = self.get_json_retry(&url)?;
        let features = value
            .get("features")
            .and_then(|v| v.as_array())
            .ok_or_else(|| StoreError::ParcelFetch("features array absent".to_string()))?;
        let mut rows = Vec::with_capacity(features.len());
        for feature in features {
            let attrs = feature.get("attributes").ok_or_else(|| {
                StoreError::ParcelFetch("feature attributes absent".to_string())
            })?;
            rows.push(attrs.clone());
        }
        Ok(rows)
    }
}

impl Default for LiveParcelSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ParcelFetch for LiveParcelSource {
    fn max_record_count(&self, layer_url: &str) -> Result<i64, StoreError> {
        let url = format!("{layer_url}?f=json");
        let value = self.get_json_retry(&url)?;
        value
            .get("maxRecordCount")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| StoreError::ParcelFetch("layer maxRecordCount absent".to_string()))
    }

    fn return_count_only(&self, layer_url: &str, where_clause: &str) -> Result<i64, StoreError> {
        let url = format!(
            "{layer_url}/query?where={}&returnCountOnly=true&f=json",
            encode_query_component(where_clause)
        );
        let value = self.get_json_retry(&url)?;
        value
            .get("count")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| StoreError::ParcelFetch("returnCountOnly count absent".to_string()))
    }

    fn distinct_prop_class(
        &self,
        layer_url: &str,
        where_clause: &str,
        prop_class_field: &str,
        taxkey_field: &str,
    ) -> Result<BTreeSet<Option<String>>, StoreError> {
        let groups = self.group_counts(layer_url, prop_class_field, where_clause, taxkey_field)?;
        Ok(groups.into_iter().map(|(key, _)| key).collect())
    }

    fn query_page(
        &self,
        layer_url: &str,
        where_clause: &str,
        offset: i64,
        page_size: i64,
        field_map: &FieldMap,
        vintage_field: Option<&str>,
        lookup_fields: Option<(&str, &str)>,
    ) -> Result<Vec<RawParcel>, StoreError> {
        let out_fields = layer_out_fields(field_map, vintage_field, lookup_fields);
        let url = format!(
            "{layer_url}/query?where={}&outFields={}&returnGeometry=false&resultOffset={}&resultRecordCount={}&f=json",
            encode_query_component(where_clause),
            encode_query_component(&out_fields),
            offset,
            page_size
        );
        let value = self.get_json_retry(&url)?;
        let features = value
            .get("features")
            .and_then(|v| v.as_array())
            .ok_or_else(|| StoreError::ParcelFetch("features array absent".to_string()))?;
        let mut rows = Vec::with_capacity(features.len());
        for feature in features {
            let attrs = feature.get("attributes").ok_or_else(|| {
                StoreError::ParcelFetch("feature attributes absent".to_string())
            })?;
            rows.push(raw_from_mapped(
                attrs,
                field_map,
                vintage_field,
                lookup_fields,
            )?);
        }
        Ok(rows)
    }
}

fn json_attr_string(attrs: &serde_json::Value, field: &str) -> Option<String> {
    match attrs.get(field) {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(other) => {
            let rendered = match other {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => other.to_string(),
            };
            let trimmed = rendered.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    }
}

fn encode_query_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn layer_out_fields(
    map: &FieldMap,
    vintage_field: Option<&str>,
    lookup_fields: Option<(&str, &str)>,
) -> String {
    let mut fields = vec![
        "OBJECTID".to_string(),
        map.taxkey.clone(),
        map.muni_code.clone(),
        map.owner_name.clone(),
    ];
    for optional in [
        map.site_address.as_deref(),
        map.mail_address_1.as_deref(),
        map.mail_address_2.as_deref(),
        map.assessed_value.as_deref(),
        map.improvement_value.as_deref(),
        map.land_value.as_deref(),
        map.prop_class.as_deref(),
        map.rec_source.as_deref(),
        vintage_field,
        lookup_fields.map(|(code, _)| code),
        lookup_fields.map(|(_, name)| name),
    ] {
        if let Some(field) = optional {
            fields.push(field.to_string());
        }
    }
    fields.sort();
    fields.dedup();
    fields.join(",")
}

fn json_attr_f64(attrs: &serde_json::Value, field: &str) -> Option<f64> {
    match attrs.get(field)? {
        serde_json::Value::Null => None,
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn mapped_string(attrs: &serde_json::Value, field: Option<&str>) -> Option<String> {
    json_attr_string(attrs, field?)
}

fn mapped_f64(attrs: &serde_json::Value, field: Option<&str>) -> Option<f64> {
    json_attr_f64(attrs, field?)
}

fn json_attr_i64(attrs: &serde_json::Value, field: &str) -> Option<i64> {
    match attrs.get(field)? {
        serde_json::Value::Null => None,
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|v| v as i64)),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn raw_from_mapped(
    attrs: &serde_json::Value,
    map: &FieldMap,
    vintage_field: Option<&str>,
    lookup_fields: Option<(&str, &str)>,
) -> Result<RawParcel, StoreError> {
    let raw_taxkey = json_attr_string(attrs, &map.taxkey).ok_or_else(|| {
        StoreError::ParcelFetch(format!("TAXKEY field {} absent", map.taxkey))
    })?;
    let taxkey = normalise_taxkey(&raw_taxkey);
    if taxkey.is_empty() {
        return Err(StoreError::ParcelFetch(
            "TAXKEY empty after normalisation".to_string(),
        ));
    }
    let export_date = vintage_field.and_then(|field| json_attr_f64(attrs, field).and_then(export_date_to_ymd));
    let owner_name = mapped_string(attrs, Some(map.owner_name.as_str()));
    let owner_type = owner_name
        .as_ref()
        .map(|name| classify_owner_type(name).to_string());
    let site_address = mapped_string(attrs, map.site_address.as_deref());
    let mail_address_1 = mapped_string(attrs, map.mail_address_1.as_deref());
    let mail_matches_site = mail_matches_site(&site_address, &mail_address_1);
    Ok(RawParcel {
        taxkey_required_normalisation: raw_taxkey != taxkey,
        taxkey,
        object_id: json_attr_i64(attrs, "OBJECTID"),
        muni_code: mapped_string(attrs, Some(map.muni_code.as_str())),
        lookup_muni_code: lookup_fields.and_then(|(code, _)| json_attr_string(attrs, code)),
        lookup_muni_name: lookup_fields.and_then(|(_, name)| json_attr_string(attrs, name)),
        owner_name,
        site_address,
        mail_address_1,
        mail_address_2: mapped_string(attrs, map.mail_address_2.as_deref()),
        assessed_value: mapped_f64(attrs, map.assessed_value.as_deref()),
        improvement_value: mapped_f64(attrs, map.improvement_value.as_deref()),
        land_value: mapped_f64(attrs, map.land_value.as_deref()),
        owner_type,
        mail_matches_site,
        prop_class: mapped_string(attrs, map.prop_class.as_deref()),
        rec_source: mapped_string(attrs, map.rec_source.as_deref()),
        value_as_of: export_date.clone(),
        export_date,
        value_source: None,
        source_layer_url: None,
    })
}

pub fn normalise_taxkey(raw: &str) -> String {
    raw.chars().filter(|c| *c != ' ').collect()
}

fn named_placeholder_key(taxkey: &str) -> Option<String> {
    let upper = taxkey.to_ascii_uppercase();
    if PLACEHOLDER_TAXKEYS.iter().any(|item| *item == upper) {
        Some(upper)
    } else {
        None
    }
}

fn is_non_parcel_key(taxkey: &str) -> bool {
    taxkey.is_empty() || !taxkey.chars().any(|c| c.is_ascii_digit())
}

fn skip_non_parcel_keys(
    rows: Vec<RawParcel>,
) -> (
    Vec<RawParcel>,
    BTreeMap<String, i64>,
    BTreeMap<String, i64>,
    BTreeMap<String, i64>,
) {
    let mut kept = Vec::with_capacity(rows.len());
    let mut known = BTreeMap::new();
    let mut unknown = BTreeMap::new();
    let mut skipped_by_muni = BTreeMap::new();
    for row in rows {
        let skipped = if let Some(key) = named_placeholder_key(&row.taxkey) {
            *known.entry(key).or_insert(0) += 1;
            true
        } else if is_non_parcel_key(&row.taxkey) {
            *unknown.entry(row.taxkey.clone()).or_insert(0) += 1;
            true
        } else {
            false
        };
        if skipped {
            let muni = row
                .muni_code
                .clone()
                .unwrap_or_else(|| "<null>".to_string());
            *skipped_by_muni.entry(muni).or_insert(0) += 1;
        } else {
            kept.push(row);
        }
    }
    (kept, known, unknown, skipped_by_muni)
}

fn format_unknown_error_text(unknown: &BTreeMap<String, i64>) -> Option<String> {
    if unknown.is_empty() {
        None
    } else {
        let parts: Vec<String> = unknown
            .iter()
            .map(|(key, count)| format!("{key}={count}"))
            .collect();
        Some(format!("skipped unknown non-keys: {}", parts.join(" ")))
    }
}

fn print_skip_report(
    known: &BTreeMap<String, i64>,
    unknown: &BTreeMap<String, i64>,
    geometry: i64,
    total: i64,
) {
    println!("known_placeholder values:");
    if known.is_empty() {
        println!("  none");
    } else {
        for (value, count) in known {
            println!("  {value}={count}");
        }
    }
    println!("unknown_non_key values:");
    if unknown.is_empty() {
        println!("  none");
    } else {
        for (value, count) in unknown {
            println!("  {value}={count}");
        }
    }
    println!("skipped_duplicate_geometry={geometry}");
    let placeholder: i64 = known.values().sum();
    let unknown_n: i64 = unknown.values().sum();
    println!(
        "skip categories placeholder={placeholder} unknown_non_key={unknown_n} duplicate_geometry={geometry} total={total}"
    );
    println!("skipped combined total={total}");
}

fn print_skipped_by_muni(skipped_by_muni: &BTreeMap<String, i64>) {
    println!("skipped per muni_code:");
    if skipped_by_muni.is_empty() {
        println!("  none");
    } else {
        for (code, count) in skipped_by_muni {
            println!("  {code}={count}");
        }
    }
}

fn mapped_field_mismatch(left: &RawParcel, right: &RawParcel) -> Option<&'static str> {
    if left.muni_code != right.muni_code {
        return Some("muni_code");
    }
    if left.owner_name != right.owner_name {
        return Some("owner_name");
    }
    if left.site_address != right.site_address {
        return Some("site_address");
    }
    if left.mail_address_1 != right.mail_address_1 {
        return Some("mail_address_1");
    }
    if left.mail_address_2 != right.mail_address_2 {
        return Some("mail_address_2");
    }
    if left.assessed_value != right.assessed_value {
        return Some("assessed_value");
    }
    if left.improvement_value != right.improvement_value {
        return Some("improvement_value");
    }
    if left.land_value != right.land_value {
        return Some("land_value");
    }
    if left.prop_class != right.prop_class {
        return Some("prop_class");
    }
    if left.rec_source != right.rec_source {
        return Some("rec_source");
    }
    None
}

fn composite_duplicate_report(rows: &[RawParcel], county_fips: &str) -> Vec<String> {
    let mut groups: BTreeMap<(String, Option<String>, String), Vec<&RawParcel>> = BTreeMap::new();
    for row in rows {
        groups
            .entry((
                county_fips.to_string(),
                row.muni_code.clone(),
                row.taxkey.clone(),
            ))
            .or_default()
            .push(row);
    }
    let mut dups: Vec<_> = groups.into_iter().filter(|(_, group)| group.len() > 1).collect();
    dups.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| a.0 .2.cmp(&b.0 .2))
    });
    let mut extra = Vec::new();
    for ((county, muni, taxkey), group) in dups.into_iter().take(10) {
        extra.push(format!(
            "composite county_fips={county} muni_code={} taxkey={taxkey} count={}",
            muni.as_deref().unwrap_or("<null>"),
            group.len()
        ));
        for row in group {
            extra.push(format!(
                "  mapped county_fips={county} muni_code={:?} taxkey={} owner_name={:?} site_address={:?} mail_address_1={:?} mail_address_2={:?} assessed_value={:?} improvement_value={:?} land_value={:?} prop_class={:?} rec_source={:?}",
                row.muni_code,
                row.taxkey,
                row.owner_name,
                row.site_address,
                row.mail_address_1,
                row.mail_address_2,
                row.assessed_value,
                row.improvement_value,
                row.land_value,
                row.prop_class,
                row.rec_source,
            ));
        }
    }
    extra
}

fn collapse_duplicate_geometry(
    rows: Vec<RawParcel>,
) -> Result<(Vec<RawParcel>, i64, BTreeMap<String, i64>), StoreError> {
    let mut by_taxkey: BTreeMap<String, Vec<RawParcel>> = BTreeMap::new();
    for row in rows {
        by_taxkey.entry(row.taxkey.clone()).or_default().push(row);
    }
    let mut kept = Vec::new();
    let mut skipped_by_muni: BTreeMap<String, i64> = BTreeMap::new();
    let mut geometry_skips = 0i64;
    let mut printed = 0usize;
    let mut affected = 0i64;
    for (taxkey, mut group) in by_taxkey {
        if group.len() == 1 {
            kept.push(group.pop().expect("one row"));
            continue;
        }
        let first = &group[0];
        for other in group.iter().skip(1) {
            if let Some(field) = mapped_field_mismatch(first, other) {
                println!(
                    "duplicate taxkey={taxkey} differs on mapped field {field}; not collapsing"
                );
                return Err(StoreError::ParcelFetch(format!(
                    "duplicate taxkey {taxkey} differs on mapped field {field}"
                )));
            }
        }
        if group.iter().any(|row| row.object_id.is_none()) {
            return Err(StoreError::ParcelFetch(format!(
                "duplicate taxkey {taxkey} missing OBJECTID; cannot keep lowest OBJECTID"
            )));
        }
        group.sort_by_key(|row| row.object_id.expect("OBJECTID present"));
        let keep = group.remove(0);
        let kept_oid = keep.object_id.expect("OBJECTID present");
        let skipped_oids: Vec<String> = group
            .iter()
            .map(|row| row.object_id.expect("OBJECTID present").to_string())
            .collect();
        affected += 1;
        if printed < 50 {
            println!(
                "duplicate_geometry taxkey={taxkey} kept_OBJECTID={kept_oid} skipped_OBJECTID={}",
                skipped_oids.join(",")
            );
            printed += 1;
        }
        for row in &group {
            let muni = row
                .muni_code
                .clone()
                .unwrap_or_else(|| "<null>".to_string());
            *skipped_by_muni.entry(muni).or_insert(0) += 1;
            geometry_skips += 1;
        }
        kept.push(keep);
    }
    if affected > 50 {
        println!(
            "duplicate_geometry taxkeys printed first 50 of {affected}"
        );
    }
    println!("duplicate_geometry affected_taxkeys={affected}");
    println!("skipped_duplicate_geometry by municipality:");
    if skipped_by_muni.is_empty() {
        println!("  none");
    } else {
        for (code, count) in &skipped_by_muni {
            println!("  {code}={count}");
        }
    }
    Ok((kept, geometry_skips, skipped_by_muni))
}

fn print_null_export_date_investigation(rows: &[RawParcel]) {
    let owner_null: Vec<&RawParcel> = rows.iter().filter(|row| row.owner_name.is_none()).collect();
    let owner_null_with_date = owner_null
        .iter()
        .filter(|row| row.export_date.is_some())
        .count();
    let owner_null_without_date = owner_null.len() - owner_null_with_date;
    let as_of_null: Vec<&RawParcel> = rows.iter().filter(|row| row.value_as_of.is_none()).collect();
    let as_of_null_with_date = as_of_null
        .iter()
        .filter(|row| row.export_date.is_some())
        .count();
    let as_of_null_without_date = as_of_null.len() - as_of_null_with_date;
    println!(
        "Task 2 owner_name null={} with Export_Date={} without Export_Date={}",
        owner_null.len(),
        owner_null_with_date,
        owner_null_without_date
    );
    println!(
        "Task 2 value_as_of null={} with Export_Date={} without Export_Date={}",
        as_of_null.len(),
        as_of_null_with_date,
        as_of_null_without_date
    );
    if as_of_null_with_date > 0 {
        println!(
            "Task 2 mapping fault: {as_of_null_with_date} rows carry Export_Date on the source and value_as_of is null"
        );
    } else if as_of_null_without_date > 0 {
        println!(
            "Task 2 source has no Export_Date for {as_of_null_without_date} value_as_of-null rows; mapping is not dropping a date; store the null"
        );
    } else {
        println!("Task 2 no value_as_of-null rows in the written set");
    }
}

pub fn classify_owner_type(name: &str) -> &'static str {
    // Fetch-time rule. Stored on entities/current_parcels.owner_type.
    // Do not change this path without a separate decision. Owner groups
    // classify from config at query time; see OwnerTypeFile::classify.
    let tokens: Vec<String> = name
        .to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_string())
        .collect();
    if is_government(&tokens) {
        return "government";
    }
    if tokens.iter().any(|t| matches!(t.as_str(), "TRUST" | "TRUSTS" | "TRST" | "TR" | "TRUSTEE" | "TRUSTEES")) {
        return "trust";
    }
    if tokens.iter().any(|t| {
        matches!(
            t.as_str(),
            "LLC"
                | "INC"
                | "INCORPORATED"
                | "CORP"
                | "CORPORATION"
                | "LP"
                | "LLP"
                | "LTD"
                | "COMPANY"
                | "PARTNERSHIP"
                | "ASSOCIATION"
                | "ASSOC"
                | "ASSOCIATES"
                | "CHURCH"
                | "BANK"
                | "HOMEOWNERS"
                | "CONDOMINIUM"
                | "CONDO"
                | "PROPERTIES"
                | "HOLDINGS"
                | "HOLDING"
                | "ENTERPRISES"
                | "FOUNDATION"
                | "AUTHORITY"
                | "CLUB"
                | "HOA"
        )
    }) {
        return "entity";
    }
    if tokens.windows(2).any(|w| w[0] == "LIMITED" && w[1] == "PARTNERSHIP") {
        return "entity";
    }
    if tokens.windows(2).any(|w| w[0] == "ESTATE" && w[1] == "OF") {
        return "entity";
    }
    "individual"
}

fn is_government(tokens: &[String]) -> bool {
    matches!(
        tokens.first().map(String::as_str),
        Some("CITY" | "VILLAGE" | "TOWN" | "COUNTY" | "STATE")
    ) && tokens.get(1).map(String::as_str) == Some("OF")
        || tokens.first().map(String::as_str) == Some("UNITED")
            && tokens.get(1).map(String::as_str) == Some("STATES")
}

fn mail_matches_site(site: &Option<String>, mail1: &Option<String>) -> Option<i64> {
    match (
        site.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        mail1.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (None, _) | (_, None) => None,
        (Some(site), Some(mail)) => Some(if site == mail { 1 } else { 0 }),
    }
}

fn export_date_to_ymd(ms: f64) -> Option<String> {
    if !ms.is_finite() {
        return None;
    }
    let secs = (ms / 1000.0).floor() as i64;
    let days = secs.div_euclid(86400);
    let (y, m, d) = ymd_from_unix_days(days);
    Some(format!("{y:04}-{m:02}-{d:02}"))
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

fn date_prefix(timestamp: &str) -> String {
    timestamp.chars().take(10).collect()
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

pub fn propclass_like_1(value: &str) -> bool {
    value.contains('1')
}

fn sql_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn muni_eq_where(muni_field: &str, muni_code: &str) -> String {
    format!("{} = '{}'", muni_field, sql_quote(muni_code))
}

fn muni_classed_where(where_clause: &str, muni_field: &str, muni_code: &str) -> String {
    format!(
        "{} AND {} = '{}'",
        where_clause,
        muni_field,
        sql_quote(muni_code)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverageAction {
    Covered,
    Fallback,
    Uncovered,
}

/// The previous rule tested for a live count of zero and
/// passed MNFV at 0.002. Genuine fallbacks sit at 0.000 and covered
/// municipalities above 0.70, the lowest observed being EGLT at 0.709.
/// The threshold sits in an empty band and is not fitted.
fn coverage_action(unfiltered: i64, classed: i64, threshold: Option<f64>) -> CoverageAction {
    match threshold {
        None => {
            if classed == 0 {
                CoverageAction::Fallback
            } else {
                CoverageAction::Covered
            }
        }
        Some(_) if unfiltered == 0 => CoverageAction::Uncovered,
        Some(limit) => {
            let ratio = classed as f64 / unfiltered as f64;
            if ratio < limit {
                CoverageAction::Fallback
            } else {
                CoverageAction::Covered
            }
        }
    }
}

struct MuniCoverage {
    code: String,
    unfiltered: i64,
    action: CoverageAction,
}

fn measure_live_coverage(
    source: &dyn ParcelFetch,
    lookup_codes: &[String],
    threshold: Option<f64>,
    primary: &Layer,
) -> Result<Vec<MuniCoverage>, StoreError> {
    match threshold {
        Some(limit) => println!("spine_coverage_fallback_threshold={limit}"),
        None => println!(
            "spine_coverage_fallback_threshold absent; using live classed count of zero"
        ),
    }
    println!("coverage table (live layer, before row fetch)");
    println!("muni_code unfiltered classed ratio fallback");
    let mut rows = Vec::with_capacity(lookup_codes.len());
    let muni_field = primary.field_map.muni_code.as_str();
    for code in lookup_codes {
        let unfiltered =
            source.return_count_only(&primary.url, &muni_eq_where(muni_field, code))?;
        let classed = source.return_count_only(
            &primary.url,
            &muni_classed_where(&primary.where_clause, muni_field, code),
        )?;
        let action = coverage_action(unfiltered, classed, threshold);
        let ratio = if unfiltered == 0 {
            None
        } else {
            Some(classed as f64 / unfiltered as f64)
        };
        let ratio_text = match ratio {
            Some(value) => format!("{value:.3}"),
            None => "n/a".to_string(),
        };
        let fallback = if action == CoverageAction::Fallback {
            "yes"
        } else {
            "no"
        };
        println!("{code} {unfiltered} {classed} {ratio_text} {fallback}");
        rows.push(MuniCoverage {
            code: code.clone(),
            unfiltered,
            action,
        });
    }
    Ok(rows)
}

fn lookup_muni_codes(conn: &Connection, county_fips: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT muni_code FROM muni_lookup WHERE county_fips = ?1 ORDER BY muni_code",
    )?;
    let rows = stmt.query_map([county_fips], |row| row.get::<_, String>(0))?;
    let mut codes = Vec::new();
    for row in rows {
        codes.push(row?);
    }
    Ok(codes)
}

fn county_lookup_row_count(conn: &Connection, county_fips: &str) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM muni_lookup WHERE county_fips = ?1",
        [county_fips],
        |row| row.get(0),
    )?)
}

fn prepare_lookup(
    tx: &Transaction<'_>,
    entity: &Entity,
    load_run_id: i64,
    assembled: &[RawParcel],
) -> Result<(), StoreError> {
    let county = entity.county_fips.as_str();
    let existing = county_lookup_row_count(tx, county)?;
    match &entity.lookup_source {
        Some(LookupSource::Layer { url, .. }) => {
            if existing == 0 {
                return Err(StoreError::ParcelFetch(format!(
                    "lookup_source Layer has no rows for county_fips={county} and live lookup fetch is not performed url={url}"
                )));
            }
            println!(
                "lookup_source Layer: existing rows={existing} for county_fips={county}; not re-fetched"
            );
            Ok(())
        }
        Some(LookupSource::Derived {
            code_field,
            name_field,
        }) => {
            println!(
                "lookup_source Derived: building from primary fields code={code_field} name={name_field} county_fips={county}"
            );
            insert_derived_lookup(tx, county, load_run_id, assembled)
        }
        None => {
            if existing == 0 {
                return Err(StoreError::ParcelFetch(format!(
                    "no lookup_source and no muni_lookup rows for county_fips={county}"
                )));
            }
            println!(
                "lookup_source absent; using {existing} existing muni_lookup rows for county_fips={county}"
            );
            Ok(())
        }
    }
}

fn insert_derived_lookup(
    tx: &Transaction<'_>,
    county_fips: &str,
    load_run_id: i64,
    assembled: &[RawParcel],
) -> Result<(), StoreError> {
    let mut by_code: BTreeMap<String, Option<String>> = BTreeMap::new();
    for row in assembled {
        let Some(code) = row.lookup_muni_code.as_ref() else {
            continue;
        };
        if code.is_empty() {
            continue;
        }
        let entry = by_code
            .entry(code.clone())
            .or_insert_with(|| row.lookup_muni_name.clone());
        if entry.is_none() {
            *entry = row.lookup_muni_name.clone();
        }
    }
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
         VALUES (?1, ?2, ?3, 'derived_primary', ?4)",
    )?;
    for (code, name) in &by_code {
        insert.execute(params![county_fips, code, name, load_run_id])?;
        println!(
            "derived lookup row muni_code={code} muni_name={}",
            name.as_deref().unwrap_or("<null>")
        );
    }
    println!(
        "derived lookup inserted distinct muni_codes={} for county_fips={county_fips} load_run_id={load_run_id}",
        by_code.len()
    );
    Ok(())
}

fn fetch_all_pages(
    source: &dyn ParcelFetch,
    layer: &Layer,
    where_clause: &str,
    page_size: i64,
    lookup_fields: Option<(&str, &str)>,
) -> Result<(Vec<RawParcel>, i64), StoreError> {
    let mut assembled = Vec::new();
    let mut offset = 0i64;
    let mut pages_fetched = 0i64;
    loop {
        let page = source.query_page(
            &layer.url,
            where_clause,
            offset,
            page_size,
            &layer.field_map,
            layer.vintage_field.as_deref(),
            lookup_fields,
        )?;
        let n = page.len() as i64;
        pages_fetched += 1;
        assembled.extend(page);
        if n < page_size {
            break;
        }
        offset += page_size;
    }
    Ok((assembled, pages_fetched))
}

fn stamp_pass(rows: &mut [RawParcel], value_source: &str, layer_url: &str) {
    for row in rows {
        row.value_source = Some(value_source.to_string());
        row.source_layer_url = Some(layer_url.to_string());
    }
}

fn layer_page_size(source: &dyn ParcelFetch, layer: &Layer) -> Result<i64, StoreError> {
    match layer.page_size {
        Some(size) => Ok(size),
        None => source.max_record_count(&layer.url),
    }
}

fn distinct_prop_classes(
    source: &dyn ParcelFetch,
    layer: &Layer,
    where_clause: &str,
) -> Result<BTreeSet<Option<String>>, StoreError> {
    let Some(prop_class_field) = layer.field_map.prop_class.as_deref() else {
        return Ok(BTreeSet::new());
    };
    source.distinct_prop_class(
        &layer.url,
        where_clause,
        prop_class_field,
        &layer.field_map.taxkey,
    )
}

fn apply_value_column(dst: &mut Option<f64>, src: Option<f64>) -> (bool, bool) {
    match (*dst, src) {
        (None, Some(value)) => {
            *dst = Some(value);
            (true, false)
        }
        (Some(_), Some(_)) => (false, true),
        _ => (false, false),
    }
}

fn apply_value_sources(
    source: &dyn ParcelFetch,
    entity: &Entity,
    live_rows: &mut [RawParcel],
) -> Result<(), StoreError> {
    let layers = entity.value_source_layers();
    if layers.is_empty() {
        println!("ValueSource layers=0; no value join");
        return Ok(());
    }
    let mut by_key: HashMap<(String, Option<String>), RawParcel> = HashMap::new();
    for layer in layers {
        println!("ValueSource fetching url={}", layer.url);
        let page_size = match layer_page_size(source, layer) {
            Ok(size) if size > 0 => {
                println!("ValueSource page size used={size} layer={}", layer.url);
                size
            }
            Ok(size) => {
                println!(
                    "ValueSource unreachable or unusable maxRecordCount={size} url={}; continuing with values null",
                    layer.url
                );
                continue;
            }
            Err(err) => {
                println!(
                    "ValueSource unreachable url={} status={err}; continuing with values null",
                    layer.url
                );
                continue;
            }
        };
        match fetch_all_pages(source, layer, &layer.where_clause, page_size, None) {
            Ok((rows, _)) => {
                println!("ValueSource rows fetched={}", rows.len());
                for row in rows {
                    by_key
                        .entry((row.taxkey.clone(), row.muni_code.clone()))
                        .or_insert(row);
                }
            }
            Err(err) => {
                println!(
                    "ValueSource fetch failed url={} status={err}; continuing with values null",
                    layer.url
                );
            }
        }
    }
    let mut took_assessed = 0i64;
    let mut took_improvement = 0i64;
    let mut took_land = 0i64;
    let mut had_assessed = 0i64;
    let mut had_improvement = 0i64;
    let mut had_land = 0i64;
    for row in live_rows.iter_mut() {
        let Some(incoming) = by_key.get(&(row.taxkey.clone(), row.muni_code.clone())) else {
            continue;
        };
        let (t, a) = apply_value_column(&mut row.assessed_value, incoming.assessed_value);
        if t {
            took_assessed += 1;
        }
        if a {
            had_assessed += 1;
        }
        let (t, a) = apply_value_column(&mut row.improvement_value, incoming.improvement_value);
        if t {
            took_improvement += 1;
        }
        if a {
            had_improvement += 1;
        }
        let (t, a) = apply_value_column(&mut row.land_value, incoming.land_value);
        if t {
            took_land += 1;
        }
        if a {
            had_land += 1;
        }
    }
    println!("ValueSource took assessed_value={took_assessed}");
    println!("ValueSource took improvement_value={took_improvement}");
    println!("ValueSource took land_value={took_land}");
    println!("ValueSource already had assessed_value={had_assessed}");
    println!("ValueSource already had improvement_value={had_improvement}");
    println!("ValueSource already had land_value={had_land}");
    print_value_nulls("assessed_value", live_rows.iter().map(|r| (r.muni_code.as_deref(), r.assessed_value)));
    print_value_nulls(
        "improvement_value",
        live_rows.iter().map(|r| (r.muni_code.as_deref(), r.improvement_value)),
    );
    print_value_nulls("land_value", live_rows.iter().map(|r| (r.muni_code.as_deref(), r.land_value)));
    Ok(())
}

fn print_value_nulls<'a, I>(column: &str, rows: I)
where
    I: Iterator<Item = (Option<&'a str>, Option<f64>)>,
{
    let mut still_null = 0i64;
    let mut by_muni: BTreeMap<String, i64> = BTreeMap::new();
    for (muni, value) in rows {
        if value.is_none() {
            still_null += 1;
            let key = muni.unwrap_or("<null>").to_string();
            *by_muni.entry(key).or_insert(0) += 1;
        }
    }
    println!("ValueSource still null {column}={still_null}");
    println!("ValueSource still-null {column} by municipality:");
    if by_muni.is_empty() {
        println!("  none");
    } else {
        for (code, count) in &by_muni {
            println!("  {code}={count}");
        }
    }
}

fn distinct_export_dates(rows: &[RawParcel]) -> Vec<String> {
    let mut dates = BTreeSet::new();
    for row in rows {
        if let Some(date) = row.export_date.as_deref() {
            dates.insert(date.to_string());
        }
    }
    dates.into_iter().collect()
}

fn count_normalised(rows: &[RawParcel]) -> i64 {
    rows.iter()
        .filter(|row| row.taxkey_required_normalisation)
        .count() as i64
}

fn lag_for_dates(dates: &[String], fetch_date: &str) -> Option<i64> {
    if dates.is_empty() {
        return None;
    }
    let mut lags: Vec<i64> = dates
        .iter()
        .map(|date| yyyymmdd_minus(fetch_date, date))
        .collect();
    lags.sort_unstable();
    lags.first().copied()
}

fn print_excluded_pass2(excluded: &[(String, Option<String>)]) {
    println!(
        "pass 2 rows excluded because taxkey already in pass 1={}",
        excluded.len()
    );
    if excluded.is_empty() {
        return;
    }
    let show = if excluded.len() <= 20 {
        excluded.len()
    } else {
        20
    };
    if excluded.len() > 20 {
        println!("excluded taxkeys (first 20 of {})", excluded.len());
    } else {
        println!("excluded taxkeys");
    }
    for (taxkey, muni) in excluded.iter().take(show) {
        println!(
            "  taxkey={taxkey} pass1_muni_code={}",
            muni.as_deref().unwrap_or("<null>")
        );
    }
}

pub fn load_parcels(
    conn: &mut Connection,
    source: &dyn ParcelFetch,
    fetched_at: &str,
    entity: &Entity,
) -> Result<ParcelLoadResult, StoreError> {
    let primary = entity.primary_layer().ok_or_else(|| {
        StoreError::ParcelFetch("entity config has no Primary layer".to_string())
    })?;
    let fallback_layer = entity.fallback_layer();
    let threshold = entity.spine_coverage_fallback_threshold;
    let primary_lookup_fields = match &entity.lookup_source {
        Some(LookupSource::Derived {
            code_field,
            name_field,
        }) => Some((code_field.as_str(), name_field.as_str())),
        _ => None,
    };
    let lookup_codes = lookup_muni_codes(conn, &entity.county_fips)?;
    println!(
        "lookup muni_codes count={} county_fips={}",
        lookup_codes.len(),
        entity.county_fips
    );

    let live_page_size = layer_page_size(source, primary)?;
    println!("pass 1 page size used={live_page_size} layer={}", primary.url);
    if live_page_size <= 0 {
        return Err(StoreError::ParcelFetch(
            "live maxRecordCount is not positive".to_string(),
        ));
    }
    let historic_page_size = if let Some(layer) = fallback_layer {
        let size = layer_page_size(source, layer)?;
        println!("pass 2 page size used={size} layer={}", layer.url);
        if size <= 0 {
            return Err(StoreError::ParcelFetch(
                "historic maxRecordCount is not positive".to_string(),
            ));
        }
        size
    } else {
        println!("pass 2 skipped: no Fallback layer in config");
        0
    };

    let coverage = measure_live_coverage(source, &lookup_codes, threshold, primary)?;
    let mut fallback = Vec::new();
    let mut uncovered = Vec::new();
    let mut live_unfiltered = BTreeMap::new();
    for row in &coverage {
        live_unfiltered.insert(row.code.clone(), row.unfiltered);
        match row.action {
            CoverageAction::Fallback => fallback.push(row.code.clone()),
            CoverageAction::Uncovered => uncovered.push(row.code.clone()),
            CoverageAction::Covered => {}
        }
    }
    if fallback_layer.is_none() {
        fallback.clear();
    }
    println!("computed fallback list={}", fallback.join(","));
    if fallback.is_empty() {
        println!("computed fallback list is empty");
        println!(
            "no municipality falls back under where={}",
            primary.where_clause
        );
        println!("run is single-pass live only; pass 2 is not executed");
    }

    let mut expected_prop_class =
        distinct_prop_classes(source, primary, &primary.where_clause)?;
    println!(
        "live layer distinct prop_class values={}",
        expected_prop_class.len()
    );
    if let Some(historic) = fallback_layer {
        let muni_field = historic.field_map.muni_code.as_str();
        for code in &fallback {
            let where_muni = muni_classed_where(&historic.where_clause, muni_field, code);
            let historic_distinct = distinct_prop_classes(source, historic, &where_muni)?;
            println!(
                "historic layer distinct prop_class values for fallback {code}={}",
                historic_distinct.len()
            );
            expected_prop_class.extend(historic_distinct);
        }
    }

    let pass1_count = source.return_count_only(&primary.url, &primary.where_clause)?;
    let like_vs_exact = pass1_count - EXACT_PROPCLASS_1_COUNT;
    println!(
        "pass 1 returnCountOnly={pass1_count} where={}",
        primary.where_clause
    );
    println!(
        "pass 1 vs exact PROPCLASS = '1' count {EXACT_PROPCLASS_1_COUNT}: difference={like_vs_exact}"
    );

    let (mut live_rows, live_pages) =
        fetch_all_pages(
            source,
            primary,
            &primary.where_clause,
            live_page_size,
            primary_lookup_fields,
        )?;
    let taxkeys_normalised_pass1 = count_normalised(&live_rows);
    println!("pass 1 taxkeys requiring space normalisation={taxkeys_normalised_pass1}");
    stamp_pass(&mut live_rows, VALUE_SOURCE_LIVE, &primary.url);
    apply_value_sources(source, entity, &mut live_rows)?;

    let live_keys: HashSet<String> = live_rows.iter().map(|row| row.taxkey.clone()).collect();
    let live_muni_by_taxkey: BTreeMap<String, Option<String>> = live_rows
        .iter()
        .map(|row| (row.taxkey.clone(), row.muni_code.clone()))
        .collect();

    // Pass 2 excludes by printed taxkey, not the composite key. Waukesha
    // live-wins across vintages: the same taxkey must not land twice
    // even when the historic row carries a different muni_code. Milwaukee
    // has no Fallback layer. Do not change this to composite or a
    // colliding Milwaukee pair would be treated as a vintage duplicate.
    let mut historic_rows = Vec::new();
    let mut pass2_server_count = 0i64;
    let mut historic_pages = 0i64;
    let mut excluded: Vec<(String, Option<String>)> = Vec::new();
    if let Some(historic) = fallback_layer {
        let muni_field = historic.field_map.muni_code.as_str();
        for code in &fallback {
            let where_muni = muni_classed_where(&historic.where_clause, muni_field, code);
            let n = source.return_count_only(&historic.url, &where_muni)?;
            println!("pass 2 historic muni {code} count={n}");
            pass2_server_count += n;
            if n == 0 {
                uncovered.push(code.clone());
                continue;
            }
            let (rows, pages) =
                fetch_all_pages(source, historic, &where_muni, historic_page_size, None)?;
            historic_pages += pages;
            for row in rows {
                if live_keys.contains(&row.taxkey) {
                    let pass1_muni = live_muni_by_taxkey.get(&row.taxkey).cloned().flatten();
                    excluded.push((row.taxkey, pass1_muni));
                } else {
                    historic_rows.push(row);
                }
            }
        }
        stamp_pass(&mut historic_rows, VALUE_SOURCE_HISTORIC, &historic.url);
    }
    println!("uncovered list={}", uncovered.join(","));
    if uncovered.is_empty() {
        println!("uncovered list is empty");
    }
    print_excluded_pass2(&excluded);
    let pass2_count = historic_rows.len() as i64;
    println!("pass 2 server returnCountOnly sum={pass2_server_count}");
    println!("pass 2 rows collected after live-wins exclusion={pass2_count}");
    let taxkeys_normalised_pass2 = count_normalised(&historic_rows);
    println!("pass 2 taxkeys requiring space normalisation={taxkeys_normalised_pass2}");

    let remaining_overlap: HashSet<&str> = historic_rows
        .iter()
        .filter(|row| live_keys.contains(&row.taxkey))
        .map(|row| row.taxkey.as_str())
        .collect();
    let cross_pass_duplicate_taxkeys = remaining_overlap.len() as i64;
    println!(
        "taxkeys remaining in both live and historic collected sets={cross_pass_duplicate_taxkeys}"
    );

    let live_export_dates = distinct_export_dates(&live_rows);
    let historic_export_dates = distinct_export_dates(&historic_rows);
    println!(
        "pass 1 Export_Date values={}",
        if live_export_dates.is_empty() {
            "<none>".to_string()
        } else {
            live_export_dates.join(",")
        }
    );
    println!(
        "pass 2 Export_Date values={}",
        if historic_export_dates.is_empty() {
            "<none>".to_string()
        } else {
            historic_export_dates.join(",")
        }
    );

    println!(
        "order of operations: fetch primary; value source join; fetch fallback; skip placeholders and unknown non-keys; collapse duplicate geometry; insert entities; insert derived lookup; run assertions"
    );

    let mut assembled = live_rows;
    assembled.extend(historic_rows);
    let fetched_count = assembled.len() as i64;
    let requested = pass1_count + pass2_count;
    let (assembled, known_placeholder, unknown_non_key, mut skipped_by_muni) =
        skip_non_parcel_keys(assembled);
    let placeholder_skips: i64 = known_placeholder.values().sum();
    let unknown_skips: i64 = unknown_non_key.values().sum();
    let (assembled, geometry_skips, geometry_by_muni) = if entity.collapse_duplicate_geometry {
        println!("collapse_duplicate_geometry=true");
        match collapse_duplicate_geometry(assembled) {
            Ok(result) => result,
            Err(err) => {
                record_failed_run(conn, &err, requested, fetched_at, &entity.spine_source_id)?;
                return Err(err);
            }
        }
    } else {
        println!("collapse_duplicate_geometry=false; duplicate-geometry skip not applied");
        (assembled, 0, BTreeMap::new())
    };
    for (code, count) in geometry_by_muni {
        *skipped_by_muni.entry(code).or_insert(0) += count;
    }
    let records_skipped = placeholder_skips + unknown_skips + geometry_skips;
    print_skip_report(
        &known_placeholder,
        &unknown_non_key,
        geometry_skips,
        records_skipped,
    );
    print_skipped_by_muni(&skipped_by_muni);
    let unknown_error_text = format_unknown_error_text(&unknown_non_key);
    print_null_export_date_investigation(&assembled);
    let assembled_count = assembled.len() as i64;
    let taxkeys_normalised = taxkeys_normalised_pass1 + taxkeys_normalised_pass2;
    let pages_fetched = live_pages + historic_pages;
    println!("pass 1 count={pass1_count} pass 2 count={pass2_count} fetched={fetched_count} skipped={records_skipped} assembled={assembled_count}");

    let fetch_date = date_prefix(fetched_at);
    let live_lag_days = lag_for_dates(&live_export_dates, &fetch_date);
    let historic_lag_days = lag_for_dates(&historic_export_dates, &fetch_date);
    let measured_lag_days = None;
    match live_lag_days {
        Some(days) => println!(
            "lag live={days} days (fetch_date={fetch_date} minus live Export_Date)"
        ),
        None => println!("lag live=ABSENT Export_Date missing on live rows"),
    }
    match historic_lag_days {
        Some(days) => println!(
            "lag historic_year_end={days} days (fetch_date={fetch_date} minus historic Export_Date)"
        ),
        None => println!("lag historic_year_end=ABSENT Export_Date missing on historic rows"),
    }
    println!("run-level measured_lag_days=null; a single column cannot hold two lags without picking one");

    println!(
        "load_run columns left null: layer_url (two layers), where_clause (1=1 live plus per-muni historic filters if fallback), value_source (live and historic_year_end), value_as_of (two Export_Date values), and no run-level lag (two lags). A single column cannot hold both without picking one."
    );

    let started_at = fetched_at.to_string();
    let finished_at = now_rfc3339();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO load_runs (
             source_id, layer_url, where_clause, value_source, value_as_of,
             started_at, finished_at, status, records_requested, records_written,
             records_skipped, error_text
         ) VALUES (
             ?1, NULL, NULL, NULL, NULL,
             ?2, ?3, 'success', ?4, ?5,
             ?6, ?7
         )",
        params![
            entity.spine_source_id,
            started_at,
            finished_at,
            requested,
            assembled_count,
            records_skipped,
            unknown_error_text
        ],
    )?;
    let load_run_id = tx.last_insert_rowid();

    let b_extra = composite_duplicate_report(&assembled, &entity.county_fips);
    if !b_extra.is_empty() {
        let mut keys = BTreeSet::new();
        for row in &assembled {
            keys.insert((
                entity.county_fips.clone(),
                row.muni_code.clone(),
                row.taxkey.clone(),
            ));
        }
        println!(
            "B distinct_composite={} written={} FAIL",
            keys.len(),
            assembled.len()
        );
        for line in &b_extra {
            println!("  {line}");
        }
        let err = StoreError::ParcelAssertion {
            code: "B".to_string(),
            expected: assembled.len().to_string(),
            actual: keys.len().to_string(),
        };
        drop(tx);
        record_failed_run(conn, &err, requested, fetched_at, &entity.spine_source_id)?;
        return Err(err);
    }

    {
        let mut insert = tx.prepare(
            "INSERT INTO entities (
                 load_run_id, county_fips, taxkey, muni_code, owner_name,
                 site_address, mail_address_1, mail_address_2, assessed_value,
                 improvement_value, land_value, owner_type, mail_matches_site,
                 prop_class, rec_source, value_source, value_as_of, source_layer_url, fetched_at
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5,
                 ?6, ?7, ?8, ?9,
                 ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19
             )",
        )?;
        for row in &assembled {
            insert.execute(params![
                load_run_id,
                entity.county_fips,
                row.taxkey,
                row.muni_code,
                row.owner_name,
                row.site_address,
                row.mail_address_1,
                row.mail_address_2,
                row.assessed_value,
                row.improvement_value,
                row.land_value,
                row.owner_type,
                row.mail_matches_site,
                row.prop_class,
                row.rec_source,
                row.value_source,
                row.value_as_of,
                row.source_layer_url,
                fetched_at,
            ])?;
        }
    }

    if let Err(err) = prepare_lookup(&tx, entity, load_run_id, &assembled) {
        drop(tx);
        record_failed_run(conn, &err, requested, fetched_at, &entity.spine_source_id)?;
        return Err(err);
    }
    println!("derived/layer lookup prepared before assertions for county_fips={}", entity.county_fips);

    if let Err(err) = run_assertions(
        &tx,
        load_run_id,
        pass1_count,
        pass2_count,
        assembled_count,
        records_skipped,
        placeholder_skips,
        unknown_skips,
        geometry_skips,
        &fallback,
        &uncovered,
        &live_unfiltered,
        &skipped_by_muni,
        &expected_prop_class,
        cross_pass_duplicate_taxkeys,
        fetched_count,
        &entity.county_fips,
    ) {
        drop(tx);
        record_failed_run(conn, &err, requested, fetched_at, &entity.spine_source_id)?;
        return Err(err);
    }

    tx.commit()?;

    Ok(ParcelLoadResult {
        load_run_id,
        status: "success".to_string(),
        records_requested: requested,
        records_written: assembled_count,
        records_skipped,
        taxkeys_normalised,
        taxkeys_normalised_pass1,
        taxkeys_normalised_pass2,
        page_size: live_page_size,
        historic_page_size,
        pages_fetched,
        pass1_count,
        pass2_count,
        fallback_codes: fallback,
        uncovered_codes: uncovered,
        live_export_dates,
        historic_export_dates,
        cross_pass_duplicate_taxkeys,
        pass2_excluded: excluded.len() as i64,
        measured_lag_days,
        live_lag_days,
        historic_lag_days,
    })
}

fn yyyymmdd_minus(later: &str, earlier: &str) -> i64 {
    let later_days = ymd_str_to_unix_days(later).unwrap_or(0);
    let earlier_days = ymd_str_to_unix_days(earlier).unwrap_or(0);
    later_days - earlier_days
}

fn ymd_str_to_unix_days(value: &str) -> Option<i64> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    Some(unix_days_from_ymd(y, m, d))
}

fn unix_days_from_ymd(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 }.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

struct Assertion {
    code: &'static str,
    expected: String,
    actual: String,
    passed: bool,
    extra: Vec<String>,
}

fn run_assertions(
    tx: &Transaction<'_>,
    load_run_id: i64,
    pass1_count: i64,
    pass2_count: i64,
    assembled: i64,
    records_skipped: i64,
    placeholder_skips: i64,
    unknown_skips: i64,
    geometry_skips: i64,
    fallback: &[String],
    uncovered: &[String],
    live_unfiltered: &BTreeMap<String, i64>,
    skipped_by_muni: &BTreeMap<String, i64>,
    expected_prop_class: &BTreeSet<Option<String>>,
    cross_pass_duplicate_taxkeys: i64,
    fetched_count: i64,
    county_fips: &str,
) -> Result<(), StoreError> {
    let mut checks = Vec::new();
    let expected_written = pass1_count + pass2_count - records_skipped;

    let written: i64 = tx.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1",
        [load_run_id],
        |row| row.get(0),
    )?;
    println!(
        "A pass1_count={pass1_count} pass2_count={pass2_count} skipped={records_skipped} written={written}"
    );
    checks.push(Assertion {
        code: "A",
        expected: format!("written=pass1+pass2-skipped={expected_written}"),
        actual: format!(
            "pass1={pass1_count} pass2={pass2_count} skipped={records_skipped} assembled={assembled} written={written}"
        ),
        passed: assembled == expected_written && written == expected_written,
        extra: Vec::new(),
    });

    let distinct_composite: i64 = tx.query_row(
        "SELECT COUNT(*) FROM (
             SELECT 1
             FROM entities
             WHERE load_run_id = ?1
             GROUP BY county_fips, muni_code, taxkey
         )",
        [load_run_id],
        |row| row.get(0),
    )?;
    let mut b_extra = Vec::new();
    if distinct_composite != written {
        let mut stmt = tx.prepare(
            "SELECT county_fips, muni_code, taxkey, COUNT(*) AS c
             FROM entities
             WHERE load_run_id = ?1
             GROUP BY county_fips, muni_code, taxkey
             HAVING c > 1
             ORDER BY c DESC, taxkey
             LIMIT 10",
        )?;
        let dup_keys: Vec<(Option<String>, Option<String>, String, i64)> = stmt
            .query_map([load_run_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        for (county, muni, taxkey, count) in &dup_keys {
            b_extra.push(format!(
                "composite county_fips={} muni_code={} taxkey={} count={count}",
                county.as_deref().unwrap_or("<null>"),
                muni.as_deref().unwrap_or("<null>"),
                taxkey
            ));
            let mut row_stmt = tx.prepare(
                "SELECT county_fips, muni_code, taxkey, owner_name, site_address,
                        mail_address_1, mail_address_2, assessed_value,
                        improvement_value, land_value, prop_class, rec_source
                 FROM entities
                 WHERE load_run_id = ?1
                   AND taxkey = ?2
                   AND county_fips IS ?3
                   AND muni_code IS ?4",
            )?;
            let mapped_rows = row_stmt.query_map(
                params![load_run_id, taxkey, county, muni],
                |row| {
                    Ok(format!(
                        "  mapped county_fips={:?} muni_code={:?} taxkey={} owner_name={:?} site_address={:?} mail_address_1={:?} mail_address_2={:?} assessed_value={:?} improvement_value={:?} land_value={:?} prop_class={:?} rec_source={:?}",
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<f64>>(7)?,
                        row.get::<_, Option<f64>>(8)?,
                        row.get::<_, Option<f64>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )?;
            for mapped in mapped_rows {
                b_extra.push(mapped?);
            }
        }
    }
    println!(
        "B distinct_composite={distinct_composite} written={written} composite keys in both live and historic passes={cross_pass_duplicate_taxkeys}"
    );
    if cross_pass_duplicate_taxkeys == 0 {
        println!("B note: no composite key appeared in both the live and historic layers");
    } else {
        println!(
            "B note: {cross_pass_duplicate_taxkeys} composite key(s) appeared in both the live and historic layers and would land twice"
        );
    }
    checks.push(Assertion {
        code: "B",
        expected: written.to_string(),
        actual: distinct_composite.to_string(),
        passed: distinct_composite == written,
        extra: b_extra,
    });

    let owner_null: i64 = tx.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND owner_name IS NULL",
        [load_run_id],
        |row| row.get(0),
    )?;
    let owner_blank: i64 = tx.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND owner_name = ''",
        [load_run_id],
        |row| row.get(0),
    )?;
    let owner_null_by_muni = column_counts(
        tx,
        load_run_id,
        "SELECT COALESCE(muni_code, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1 AND owner_name IS NULL
         GROUP BY muni_code
         ORDER BY 1",
    )?;
    println!("C owner-less parcels={owner_null} blank={owner_blank}");
    println!("C owner-less muni_code distribution:");
    if owner_null_by_muni.is_empty() {
        println!("  none");
    } else {
        for (code, count) in &owner_null_by_muni {
            println!("  {code}={count}");
        }
    }
    checks.push(Assertion {
        code: "C",
        expected: "owner-less parcels printed, not failed".to_string(),
        actual: format!("null={owner_null} blank={owner_blank}"),
        passed: true,
        extra: Vec::new(),
    });

    let imp_null: i64 = tx.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND improvement_value IS NULL",
        [load_run_id],
        |row| row.get(0),
    )?;
    let imp_null_by_muni = column_counts(
        tx,
        load_run_id,
        "SELECT COALESCE(muni_code, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1 AND improvement_value IS NULL
         GROUP BY muni_code
         ORDER BY 1",
    )?;
    let imp_fraction = if written == 0 {
        0.0
    } else {
        imp_null as f64 / written as f64
    };
    println!(
        "D improvement_value null={imp_null} written={written} fraction={imp_fraction:.3}"
    );
    println!("D improvement_value null muni_code distribution:");
    if imp_null_by_muni.is_empty() {
        println!("  none");
    } else {
        for (code, count) in &imp_null_by_muni {
            println!("  {code}={count}");
        }
    }
    checks.push(Assertion {
        code: "D",
        expected: "improvement_value null printed, not failed".to_string(),
        actual: format!("null={imp_null} fraction={imp_fraction:.3}"),
        passed: true,
        extra: Vec::new(),
    });

    // Original assertion E was one-directional: it required every fetched
    // muni_code to resolve in muni_lookup and ignored the reverse. That
    // check passed on load_run 2, which was missing seven municipalities
    // (BRC, LLV, MILC, MSKC, OCOT, WAKC, WAKV) because PROPCLASS = '1'
    // excludes whole municipalities rather than a random sample. A
    // one-directional join check is how a filter's exclusion renders as
    // an empty set: the omitted codes never appear in the fetched rows,
    // so they never fail the entity-to-lookup side.
    let entity_muni: i64 = tx.query_row(
        "SELECT COUNT(DISTINCT muni_code) FROM entities WHERE load_run_id = ?1 AND muni_code IS NOT NULL",
        [load_run_id],
        |row| row.get(0),
    )?;
    let lookup_muni: i64 = tx.query_row(
        "SELECT COUNT(DISTINCT muni_code) FROM muni_lookup WHERE county_fips = ?1",
        [county_fips],
        |row| row.get(0),
    )?;
    let e1_matched: i64 = tx.query_row(
        "SELECT COUNT(DISTINCT e.muni_code)
         FROM entities e
         INNER JOIN muni_lookup m
            ON m.muni_code = e.muni_code
           AND m.county_fips = ?2
         WHERE e.load_run_id = ?1
           AND e.muni_code IS NOT NULL",
        params![load_run_id, county_fips],
        |row| row.get(0),
    )?;
    let mut unmatched_entities = Vec::new();
    {
        let mut unmatched_stmt = tx.prepare(
            "SELECT DISTINCT e.muni_code
             FROM entities e
             WHERE e.load_run_id = ?1
               AND e.muni_code IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM muni_lookup m
                   WHERE m.muni_code = e.muni_code
                     AND m.county_fips = ?2
               )
             ORDER BY e.muni_code",
        )?;
        let rows = unmatched_stmt.query_map(params![load_run_id, county_fips], |row| {
            row.get::<_, String>(0)
        })?;
        for row in rows {
            unmatched_entities.push(format!("unmatched fetched muni_code={}", row?));
        }
    }
    println!(
        "E1 county_fips={county_fips} matched/distinct fetched={e1_matched}/{entity_muni} matched/distinct lookup={e1_matched}/{lookup_muni}"
    );
    checks.push(Assertion {
        code: "E1",
        expected: format!(
            "all {entity_muni} fetched muni_codes resolve in lookup for county_fips={county_fips}"
        ),
        actual: format!("matched={e1_matched} unmatched={}", unmatched_entities.len()),
        passed: unmatched_entities.is_empty() && e1_matched == entity_muni,
        extra: unmatched_entities,
    });

    let e2_matched: i64 = tx.query_row(
        "SELECT COUNT(DISTINCT m.muni_code)
         FROM muni_lookup m
         INNER JOIN entities e ON e.muni_code = m.muni_code
         WHERE e.load_run_id = ?1
           AND e.muni_code IS NOT NULL
           AND m.county_fips = ?2",
        params![load_run_id, county_fips],
        |row| row.get(0),
    )?;
    println!("E2 uncovered list={}", uncovered.join(","));
    let uncovered_set: HashSet<&str> = uncovered.iter().map(String::as_str).collect();
    let mut missing_lookup = Vec::new();
    {
        let mut missing_stmt = tx.prepare(
            "SELECT m.muni_code
             FROM muni_lookup m
             WHERE m.county_fips = ?2
               AND NOT EXISTS (
                 SELECT 1 FROM entities e
                 WHERE e.load_run_id = ?1
                   AND e.muni_code = m.muni_code
             )
             ORDER BY m.muni_code",
        )?;
        let rows = missing_stmt.query_map(params![load_run_id, county_fips], |row| {
            row.get::<_, String>(0)
        })?;
        for row in rows {
            let code = row?;
            if uncovered_set.contains(code.as_str()) {
                continue;
            }
            missing_lookup.push(code);
        }
    }
    println!(
        "E2 county_fips={county_fips} matched/distinct lookup={e2_matched}/{lookup_muni} matched/distinct fetched={e2_matched}/{entity_muni}"
    );
    if missing_lookup.iter().any(|code| code == "MILC") {
        println!("E2 MILC still returns zero; that is a real coverage fact and it fails");
    }
    let missing_list = missing_lookup.join(",");
    let mut e2_extra: Vec<String> = missing_lookup
        .iter()
        .map(|code| format!("missing lookup muni_code={code}"))
        .collect();
    if !missing_lookup.is_empty() {
        e2_extra.insert(0, format!("missing lookup codes {missing_list}"));
    }
    checks.push(Assertion {
        code: "E2",
        expected: format!(
            "every lookup muni_code for county_fips={county_fips} appears in fetched rows, except uncovered"
        ),
        actual: if missing_lookup.is_empty() {
            format!("matched={e2_matched}")
        } else {
            format!("matched={e2_matched} missing={missing_list}")
        },
        passed: missing_lookup.is_empty(),
        extra: e2_extra,
    });

    let mut bad_taxkeys = Vec::new();
    let mut bad_taxkey = 0i64;
    {
        let mut stmt = tx.prepare("SELECT taxkey FROM entities WHERE load_run_id = ?1")?;
        let rows = stmt.query_map([load_run_id], |row| row.get::<_, String>(0))?;
        for row in rows {
            let taxkey = row?;
            if taxkey.chars().any(|c| c == ' ' || !c.is_ascii_alphanumeric()) {
                bad_taxkey += 1;
                if bad_taxkeys.len() < 10 {
                    bad_taxkeys.push(format!("taxkey={taxkey}"));
                }
            }
        }
    }
    checks.push(Assertion {
        code: "F",
        expected: "0".to_string(),
        actual: bad_taxkey.to_string(),
        passed: bad_taxkey == 0,
        extra: bad_taxkeys,
    });

    let source_null: i64 = tx.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND value_source IS NULL",
        [load_run_id],
        |row| row.get(0),
    )?;
    let as_of_null: i64 = tx.query_row(
        "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND value_as_of IS NULL",
        [load_run_id],
        |row| row.get(0),
    )?;
    println!("G value_as_of null={as_of_null} printed, not failed");
    let as_of_null_by_muni = column_counts(
        tx,
        load_run_id,
        "SELECT COALESCE(muni_code, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1 AND value_as_of IS NULL
         GROUP BY muni_code
         ORDER BY 1",
    )?;
    println!("G value_as_of null muni_code distribution:");
    if as_of_null_by_muni.is_empty() {
        println!("  none");
    } else {
        for (code, count) in &as_of_null_by_muni {
            println!("  {code}={count}");
        }
    }
    checks.push(Assertion {
        code: "G",
        expected: "value_source null=0".to_string(),
        actual: format!("value_source_null={source_null} value_as_of_null={as_of_null}"),
        passed: source_null == 0,
        extra: Vec::new(),
    });

    let written_by_muni = muni_counts(tx, load_run_id)?;
    let fallback_set: BTreeSet<&str> = fallback.iter().map(String::as_str).collect();
    let uncovered_for_h: BTreeSet<&str> = uncovered.iter().map(String::as_str).collect();
    let mut h_extra = Vec::new();
    let mut h_failed = Vec::new();
    // Assertion H failed on load_run 8 for Ottawa Town (OCOT) at 17
    // written of 18 unfiltered. The cause was comparing against a
    // count that included rows the skip rule had already removed.
    // The 5 percent threshold was not widened: that would blind the
    // check on small municipalities, which is where a paging failure
    // would be hardest to see.
    println!("H per municipality: unfiltered skipped expected written divergence");
    for (code, unfiltered) in live_unfiltered {
        let skipped_muni = skipped_by_muni.get(code).copied().unwrap_or(0);
        let expected = unfiltered - skipped_muni;
        let written_muni = written_by_muni.get(code).copied().unwrap_or(0);
        let divergence = if expected == 0 {
            if written_muni == 0 {
                0.0
            } else {
                1.0
            }
        } else {
            (written_muni - expected).abs() as f64 / expected as f64
        };
        println!(
            "  {code} unfiltered={unfiltered} skipped={skipped_muni} expected={expected} written={written_muni} divergence={divergence:.3}"
        );
        if fallback_set.contains(code.as_str()) || uncovered_for_h.contains(code.as_str()) {
            continue;
        }
        let within = within_five_percent(written_muni, expected);
        let line = format!(
            "{code} unfiltered={unfiltered} skipped={skipped_muni} expected={expected} written={written_muni}"
        );
        h_extra.push(line.clone());
        if !within {
            h_failed.push(line);
        }
    }
    checks.push(Assertion {
        code: "H",
        expected: "each non-fallback municipality written count within 5 percent of live unfiltered count minus skipped in that municipality"
            .to_string(),
        actual: if h_failed.is_empty() {
            format!("checked={} diverged=0", h_extra.len())
        } else {
            format!("checked={} diverged={}", h_extra.len(), h_failed.len())
        },
        passed: h_failed.is_empty(),
        extra: if h_failed.is_empty() {
            Vec::new()
        } else {
            h_failed
        },
    });

    let mut written_prop_class = BTreeSet::new();
    {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT prop_class FROM entities WHERE load_run_id = ?1",
        )?;
        let rows = stmt.query_map([load_run_id], |row| row.get::<_, Option<String>>(0))?;
        for row in rows {
            written_prop_class.insert(row?);
        }
    }
    let mut missing_prop: Vec<String> = expected_prop_class
        .difference(&written_prop_class)
        .map(|v| format!("missing {}", display_prop_class(v)))
        .collect();
    let extra_prop: Vec<String> = written_prop_class
        .difference(expected_prop_class)
        .map(|v| format!("extra {}", display_prop_class(v)))
        .collect();
    missing_prop.extend(extra_prop.iter().cloned());
    println!(
        "I written distinct prop_class={} profile distinct prop_class={}",
        written_prop_class.len(),
        expected_prop_class.len()
    );
    checks.push(Assertion {
        code: "I",
        expected: format!(
            "written distinct prop_class matches profile distinct ({})",
            expected_prop_class.len()
        ),
        actual: format!(
            "written={} profile={} mismatched={}",
            written_prop_class.len(),
            expected_prop_class.len(),
            missing_prop.len()
        ),
        passed: written_prop_class == *expected_prop_class,
        extra: missing_prop,
    });

    let mut placeholder_written = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT DISTINCT taxkey FROM entities WHERE load_run_id = ?1")?;
        let rows = stmt.query_map([load_run_id], |row| row.get::<_, String>(0))?;
        for row in rows {
            let taxkey = row?;
            if named_placeholder_key(&taxkey).is_some() {
                placeholder_written.push(taxkey);
            }
        }
    }
    checks.push(Assertion {
        code: "J",
        expected: "no written taxkey is a named placeholder".to_string(),
        actual: if placeholder_written.is_empty() {
            "placeholder_written=0".to_string()
        } else {
            format!("placeholder_written={}", placeholder_written.join(","))
        },
        passed: placeholder_written.is_empty(),
        extra: placeholder_written
            .iter()
            .map(|taxkey| format!("taxkey={taxkey}"))
            .collect(),
    });

    println!(
        "K written={written} placeholder_skips={placeholder_skips} unknown_non_key_skips={unknown_skips} duplicate_geometry_skips={geometry_skips} skipped_total={records_skipped} fetched={fetched_count}"
    );
    checks.push(Assertion {
        code: "K",
        expected: format!(
            "written+placeholder+unknown_non_key+duplicate_geometry=fetched={fetched_count}"
        ),
        actual: format!(
            "written={written} placeholder={placeholder_skips} unknown_non_key={unknown_skips} duplicate_geometry={geometry_skips} sum={} fetched={fetched_count}",
            written + placeholder_skips + unknown_skips + geometry_skips
        ),
        passed: written + placeholder_skips + unknown_skips + geometry_skips == fetched_count,
        extra: Vec::new(),
    });

    let mut failed_code = None;
    for check in &checks {
        let result = if check.passed { "PASS" } else { "FAIL" };
        println!(
            "{} expected={} actual={} {}",
            check.code, check.expected, check.actual, result
        );
        for line in &check.extra {
            println!("  {line}");
        }
        if !check.passed && failed_code.is_none() {
            failed_code = Some(check);
        }
    }
    if let Some(check) = failed_code {
        return Err(StoreError::ParcelAssertion {
            code: check.code.to_string(),
            expected: check.expected.clone(),
            actual: check.actual.clone(),
        });
    }

    crate::current::replace_run_keys(tx, load_run_id, county_fips)?;
    crate::current::assert_agrees_with_view(tx, county_fips)?;
    Ok(())
}

fn column_counts(
    tx: &Transaction<'_>,
    load_run_id: i64,
    sql: &str,
) -> Result<Vec<(String, i64)>, StoreError> {
    let mut stmt = tx.prepare(sql)?;
    let rows = stmt.query_map([load_run_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn display_prop_class(value: &Option<String>) -> String {
    match value {
        Some(v) => format!("prop_class={v}"),
        None => "prop_class=<null>".to_string(),
    }
}

fn within_five_percent(written: i64, classed: i64) -> bool {
    if classed == 0 {
        written == 0
    } else {
        (written - classed).abs() as f64 <= classed as f64 * 0.05
    }
}

fn record_failed_run(
    conn: &Connection,
    err: &StoreError,
    server_count: i64,
    fetched_at: &str,
    source_id: &str,
) -> Result<(), StoreError> {
    let error_text = match err {
        StoreError::ParcelAssertion { code, actual, .. } if code == "E2" => {
            format!("failed assertion E2: {actual}")
        }
        StoreError::ParcelAssertion { code, actual, .. } if code == "placeholder" => {
            format!("failed assertion placeholder: {actual}")
        }
        StoreError::ParcelAssertion { code, .. } => format!("failed assertion {code}"),
        other => other.to_string(),
    };
    let finished_at = now_rfc3339();
    conn.execute(
        "INSERT INTO load_runs (
             source_id, layer_url, where_clause, value_source, value_as_of,
             started_at, finished_at, status, records_requested, records_written,
             records_skipped, error_text
         ) VALUES (
             ?1, NULL, NULL, NULL, NULL,
             ?2, ?3, 'failed', ?4, 0,
             0, ?5
         )",
        params![source_id, fetched_at, finished_at, server_count, error_text],
    )?;
    Ok(())
}

pub fn print_load_run(conn: &Connection, load_run_id: i64) -> Result<(), StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, source_id, layer_url, where_clause, value_source, value_as_of,
                started_at, finished_at, status, records_requested, records_written,
                records_skipped, error_text
         FROM load_runs WHERE id = ?1",
    )?;
    stmt.query_row([load_run_id], |row| {
        println!("load_run id={}", row.get::<_, i64>(0)?);
        println!("  source_id={}", row.get::<_, String>(1)?);
        println!("  layer_url={:?}", row.get::<_, Option<String>>(2)?);
        println!("  where_clause={:?}", row.get::<_, Option<String>>(3)?);
        println!("  value_source={:?}", row.get::<_, Option<String>>(4)?);
        println!("  value_as_of={:?}", row.get::<_, Option<String>>(5)?);
        println!("  started_at={:?}", row.get::<_, Option<String>>(6)?);
        println!("  finished_at={:?}", row.get::<_, Option<String>>(7)?);
        println!("  status={}", row.get::<_, String>(8)?);
        println!("  records_requested={:?}", row.get::<_, Option<i64>>(9)?);
        println!("  records_written={:?}", row.get::<_, Option<i64>>(10)?);
        println!("  records_skipped={:?}", row.get::<_, Option<i64>>(11)?);
        println!("  error_text={:?}", row.get::<_, Option<String>>(12)?);
        Ok(())
    })?;
    Ok(())
}

pub fn print_latest_load_run(conn: &Connection) -> Result<(), StoreError> {
    let id: Option<i64> = conn.query_row("SELECT MAX(id) FROM load_runs", [], |row| row.get(0))?;
    match id {
        Some(id) => print_load_run(conn, id),
        None => {
            println!("load_run: none");
            Ok(())
        }
    }
}

pub fn compare_load_runs(conn: &Connection, run_a: i64, run_b: i64) -> Result<(), StoreError> {
    compare_load_runs_opts(conn, run_a, run_b, true)
}

pub fn compare_load_runs_opts(
    conn: &Connection,
    run_a: i64,
    run_b: i64,
    include_government: bool,
) -> Result<(), StoreError> {
    ensure_compare_index(conn)?;

    let count_a = with_query_timeout(conn, "row_count_run_a", |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1",
            [run_a],
            |row| row.get::<_, i64>(0),
        )
        .map_err(StoreError::from)
    })?;
    if let Some(count_a) = count_a {
        println!("compare row count run {run_a}={count_a}");
    }

    let count_b = with_query_timeout(conn, "row_count_run_b", |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1",
            [run_b],
            |row| row.get::<_, i64>(0),
        )
        .map_err(StoreError::from)
    })?;
    if let Some(count_b) = count_b {
        println!("compare row count run {run_b}={count_b}");
    }
    match (count_a, count_b) {
        (Some(a), Some(b)) => {
            let gap = a - b;
            println!("compare row count gap run {run_a} minus run {run_b}={gap}");
        }
        _ => {}
    }

    let munis_a = with_query_timeout(conn, "muni_counts_run_a", |conn| muni_counts(conn, run_a))?;
    if let Some(ref munis_a) = munis_a {
        println!("muni_code counts run {run_a} fetched {}", munis_a.len());
    }
    let munis_b = with_query_timeout(conn, "muni_counts_run_b", |conn| muni_counts(conn, run_b))?;
    if let Some(ref munis_b) = munis_b {
        println!("muni_code counts run {run_b} fetched {}", munis_b.len());
    }
    if let (Some(munis_a), Some(munis_b)) = (munis_a, munis_b) {
        let mut codes = BTreeSet::new();
        codes.extend(munis_a.keys().cloned());
        codes.extend(munis_b.keys().cloned());
        println!("muni_code counts run {run_a} vs run {run_b}");
        let mut over_100 = Vec::new();
        for code in &codes {
            let a = munis_a.get(code).copied().unwrap_or(0);
            let b = munis_b.get(code).copied().unwrap_or(0);
            let delta = b - a;
            println!("  {code} run {run_a}={a} run {run_b}={b} delta={delta}");
            if delta.abs() > 100 {
                over_100.push((code.clone(), a, b, delta));
            }
        }
        println!("municipalities diverging by more than 100 rows:");
        if over_100.is_empty() {
            println!("  none");
        } else {
            for (code, a, b, delta) in &over_100 {
                println!("  {code} run {run_a}={a} run {run_b}={b} delta={delta}");
            }
        }
    }

    let keys_a = with_query_timeout(conn, "taxkeys_run_a", |conn| load_taxkeys(conn, run_a))?;
    if let Some(ref keys_a) = keys_a {
        println!("taxkeys run {run_a} fetched {}", keys_a.len());
    }
    let keys_b = with_query_timeout(conn, "taxkeys_run_b", |conn| load_taxkeys(conn, run_b))?;
    if let Some(ref keys_b) = keys_b {
        println!("taxkeys run {run_b} fetched {}", keys_b.len());
    }
    if let (Some(keys_a), Some(keys_b)) = (keys_a, keys_b) {
        let only_a = keys_a.difference(&keys_b).count();
        println!("taxkeys in run {run_a} absent from run {run_b}={only_a}");
        let only_b = keys_b.difference(&keys_a).count();
        println!("taxkeys in run {run_b} absent from run {run_a}={only_b}");
    }

    let composite_a = with_query_timeout(conn, "composite_run_a", |conn| {
        load_composite_keys(conn, run_a)
    })?;
    let composite_b = with_query_timeout(conn, "composite_run_b", |conn| {
        load_composite_keys(conn, run_b)
    })?;
    if let (Some(a), Some(b)) = (composite_a, composite_b) {
        println!("composite keys run {run_a}={}", a.len());
        println!("composite keys run {run_b}={}", b.len());
        println!(
            "composite keys in run {run_a} absent from run {run_b}={}",
            a.difference(&b).count()
        );
        println!(
            "composite keys in run {run_b} absent from run {run_a}={}",
            b.difference(&a).count()
        );
    }

    let owner_a = with_query_timeout(conn, "owner_type_run_a", |conn| {
        owner_type_distribution(conn, run_a)
    })?;
    if let Some(dist) = owner_a {
        println!("owner_type distribution run {run_a}: {dist}");
    }
    let owner_b = with_query_timeout(conn, "owner_type_run_b", |conn| {
        owner_type_distribution(conn, run_b)
    })?;
    if let Some(dist) = owner_b {
        println!("owner_type distribution run {run_b}: {dist}");
    }

    let prop_b = with_query_timeout(conn, "prop_class_top20_run_b", |conn| {
        prop_class_top(conn, run_b, 20)
    })?;
    if let Some(rows) = prop_b {
        println!("prop_class distribution run {run_b} top twenty:");
        for (value, count) in rows {
            println!("  {value}={count}");
        }
    }
    let prop_null = with_query_timeout(conn, "prop_class_null_run_b", |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND prop_class IS NULL",
            [run_b],
            |row| row.get::<_, i64>(0),
        )
        .map_err(StoreError::from)
    })?;
    if let Some(n) = prop_null {
        println!("prop_class null count run {run_b}={n}");
    }

    let imp_null_total = with_query_timeout(conn, "improvement_null_count_run_b", |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1 AND improvement_value IS NULL",
            [run_b],
            |row| row.get::<_, i64>(0),
        )
        .map_err(StoreError::from)
    })?;
    if let Some(n) = imp_null_total {
        println!("improvement_value null count run {run_b}={n}");
    }

    let imp_null_class = with_query_timeout(conn, "improvement_null_by_prop_class_run_b", |conn| {
        improvement_null_by_prop_class(conn, run_b, 10)
    })?;
    if let Some(rows) = imp_null_class {
        println!("improvement_value null count by prop_class run {run_b} top ten:");
        for (value, count) in rows {
            println!("  {value}={count}");
        }
    }

    let source_b = with_query_timeout(conn, "value_source_run_b", |conn| {
        value_source_distribution(conn, run_b)
    })?;
    if let Some(dist) = source_b {
        println!("value_source distribution run {run_b}: {dist}");
    }

    if include_government {
        let gov_a = with_query_timeout(conn, "government_parcels_run_a", |conn| {
            government_parcels(conn, run_a)
        })?;
        if let Some(gov_a) = gov_a {
            println!(
                "owner_type government parcels in run {run_a} count={}",
                gov_a.len()
            );
            for (taxkey, muni_a) in gov_a {
                let in_b = with_query_timeout(conn, &format!("government_taxkey_{taxkey}_run_b"), |conn| {
                    taxkey_muni_in_run(conn, run_b, &taxkey)
                })?;
                match in_b {
                    Some(Some(muni_b)) => println!(
                        "  taxkey={taxkey} run {run_a} muni_code={} present in run {run_b} muni_code={}",
                        muni_a.as_deref().unwrap_or("<null>"),
                        muni_b.as_deref().unwrap_or("<null>")
                    ),
                    Some(None) => println!(
                        "  taxkey={taxkey} run {run_a} muni_code={} absent from run {run_b}",
                        muni_a.as_deref().unwrap_or("<null>")
                    ),
                    None => {}
                }
            }
        }
    }
    Ok(())
}

fn query_is_interrupted(err: &StoreError) -> bool {
    matches!(
        err,
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::OperationInterrupted
    )
}

fn with_query_timeout<T>(
    conn: &Connection,
    name: &str,
    f: impl FnOnce(&Connection) -> Result<T, StoreError>,
) -> Result<Option<T>, StoreError> {
    println!("compare query start: {name}");
    let handle = conn.get_interrupt_handle();
    let (done_tx, done_rx) = mpsc::channel();
    let watcher = thread::spawn(move || {
        if done_rx.recv_timeout(COMPARE_QUERY_TIMEOUT).is_err() {
            handle.interrupt();
        }
    });
    let result = f(conn);
    let _ = done_tx.send(());
    let _ = watcher.join();
    match result {
        Ok(value) => Ok(Some(value)),
        Err(err) if query_is_interrupted(&err) => {
            println!("TIMED OUT {name}");
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

fn ensure_compare_index(conn: &Connection) -> Result<(), StoreError> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'index' AND name = 'entities_load_run_taxkey'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        println!(
            "compare creating index on (load_run_id, taxkey); SQLite cannot CREATE TEMP INDEX on a main table"
        );
        conn.execute(
            "CREATE INDEX IF NOT EXISTS entities_load_run_taxkey ON entities (load_run_id, taxkey)",
            [],
        )?;
    } else {
        println!("compare index on (load_run_id, taxkey) already present");
    }
    Ok(())
}

fn load_taxkeys(conn: &Connection, load_run_id: i64) -> Result<BTreeSet<String>, StoreError> {
    let mut stmt = conn.prepare("SELECT taxkey FROM entities WHERE load_run_id = ?1")?;
    let rows = stmt.query_map([load_run_id], |row| row.get::<_, String>(0))?;
    let mut set = BTreeSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set)
}

fn load_composite_keys(
    conn: &Connection,
    load_run_id: i64,
) -> Result<BTreeSet<(Option<String>, Option<String>, String)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT county_fips, muni_code, taxkey FROM entities WHERE load_run_id = ?1",
    )?;
    let rows = stmt.query_map([load_run_id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    let mut set = BTreeSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set)
}

fn prop_class_top(
    conn: &Connection,
    load_run_id: i64,
    limit: i64,
) -> Result<Vec<(String, i64)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(prop_class, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1
         GROUP BY prop_class
         ORDER BY COUNT(*) DESC, 1
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![load_run_id, limit], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn improvement_null_by_prop_class(
    conn: &Connection,
    load_run_id: i64,
    limit: i64,
) -> Result<Vec<(String, i64)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(prop_class, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1 AND improvement_value IS NULL
         GROUP BY prop_class
         ORDER BY COUNT(*) DESC, 1
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![load_run_id, limit], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn owner_type_distribution(conn: &Connection, load_run_id: i64) -> Result<String, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(owner_type, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1
         GROUP BY owner_type
         ORDER BY 1",
    )?;
    let rows = stmt.query_map([load_run_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows
        .map(|row| row.map(|(name, count)| format!("{name}={count}")))
        .collect::<Result<Vec<_>, _>>()?
        .join(" "))
}

fn government_parcels(
    conn: &Connection,
    load_run_id: i64,
) -> Result<Vec<(String, Option<String>)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT taxkey, muni_code
         FROM entities
         WHERE load_run_id = ?1 AND owner_type = 'government'
         ORDER BY taxkey",
    )?;
    let rows = stmt.query_map([load_run_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn taxkey_muni_in_run(
    conn: &Connection,
    load_run_id: i64,
    taxkey: &str,
) -> Result<Option<Option<String>>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT muni_code FROM entities WHERE load_run_id = ?1 AND taxkey = ?2 LIMIT 1",
    )?;
    let mut rows = stmt.query(params![load_run_id, taxkey])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

fn muni_counts(conn: &Connection, load_run_id: i64) -> Result<BTreeMap<String, i64>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(muni_code, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1
         GROUP BY muni_code
         ORDER BY 1",
    )?;
    let rows = stmt.query_map([load_run_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut map = BTreeMap::new();
    for row in rows {
        let (code, count) = row?;
        map.insert(code, count);
    }
    Ok(map)
}

fn value_source_distribution(conn: &Connection, load_run_id: i64) -> Result<String, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(value_source, '<null>'), COUNT(*)
         FROM entities
         WHERE load_run_id = ?1
         GROUP BY value_source
         ORDER BY 1",
    )?;
    let rows = stmt.query_map([load_run_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows
        .map(|row| row.map(|(name, count)| format!("{name}={count}")))
        .collect::<Result<Vec<_>, _>>()?
        .join(" "))
}

pub fn print_current_view_check(conn: &Connection) -> Result<(), StoreError> {
    let current_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM current_entities", [], |row| row.get(0))?;
    let distinct_across: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (
             SELECT e.county_fips, e.muni_code, e.taxkey
             FROM entities e
             INNER JOIN load_runs r ON r.id = e.load_run_id
             WHERE r.status = 'success'
             GROUP BY e.county_fips, e.muni_code, e.taxkey
         )",
        [],
        |row| row.get(0),
    )?;
    println!(
        "current_entities count={current_count} distinct composite keys across all successful runs={distinct_across}"
    );
    if current_count == distinct_across {
        println!("current_entities is one row per county_fips, muni_code, taxkey");
    } else {
        println!("current_entities is NOT one row per county_fips, muni_code, taxkey");
    }

    let mut stmt = conn.prepare(
        "SELECT COALESCE(load_run_id, -1), COUNT(*)
         FROM current_entities
         GROUP BY load_run_id
         ORDER BY load_run_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    println!("current_entities resolving by load_run:");
    for row in rows {
        let (run_id, count) = row?;
        println!("  load_run {run_id}={count}");
    }

    let mut county_stmt = conn.prepare(
        "SELECT COALESCE(county_fips, '<null>'), COUNT(*)
         FROM current_entities
         GROUP BY county_fips
         ORDER BY 1",
    )?;
    let county_rows = county_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    println!("current_entities by county_fips:");
    for row in county_rows {
        let (county, count) = row?;
        println!("  {county}={count}");
    }

    let collisions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (
             SELECT taxkey
             FROM entities e
             INNER JOIN load_runs r ON r.id = e.load_run_id
             WHERE r.status = 'success'
             GROUP BY taxkey
             HAVING COUNT(DISTINCT e.county_fips) > 1
         )",
        [],
        |row| row.get(0),
    )?;
    println!("taxkeys present in more than one county_fips={collisions}");
    if collisions > 0 {
        let mut stmt = conn.prepare(
            "SELECT taxkey, GROUP_CONCAT(DISTINCT county_fips)
             FROM entities e
             INNER JOIN load_runs r ON r.id = e.load_run_id
             WHERE r.status = 'success'
             GROUP BY taxkey
             HAVING COUNT(DISTINCT e.county_fips) > 1
             ORDER BY taxkey
             LIMIT 20",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        println!("colliding taxkeys (first 20):");
        for row in rows {
            let (taxkey, counties) = row?;
            println!("  taxkey={taxkey} counties={counties}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::migrate;
    use radar_config::LayerRole;

    const PARCEL_LAYER_URL: &str =
        "https://gis.waukeshacounty.gov/host/rest/services/Web_Tax_Parcel/FeatureServer/2";
    const WAUKESHA_HISTORIC: &str =
        "https://gis.waukeshacounty.gov/host/rest/services/Historic_Parcels/FeatureServer/1";
    const VALUE_SOURCE_URL: &str = "https://example.invalid/value-source";

    fn waukesha_field_map() -> FieldMap {
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

    fn waukesha_layer(role: LayerRole, url: &str) -> Layer {
        Layer {
            role,
            url: url.to_string(),
            where_clause: "1=1".to_string(),
            page_size: None,
            vintage_field: Some("Export_Date".to_string()),
            field_map: waukesha_field_map(),
        }
    }

    fn waukesha_entity(threshold: Option<f64>) -> Entity {
        Entity {
            spine_source_id: "src-waukesha-parcels".to_string(),
            county_fips: "55133".to_string(),
            natural_key_field: Some("taxkey".to_string()),
            has_stable_key: true,
            spine_coverage_fallback_threshold: threshold,
            layers: vec![
                waukesha_layer(LayerRole::Primary, PARCEL_LAYER_URL),
                waukesha_layer(LayerRole::Fallback, WAUKESHA_HISTORIC),
            ],
            attachments: Vec::new(),
            lookup_source: None,
            collapse_duplicate_geometry: false,
        }
    }

    fn load_parcels(
        conn: &mut Connection,
        source: &dyn ParcelFetch,
        fetched_at: &str,
    ) -> Result<ParcelLoadResult, StoreError> {
        super::load_parcels(conn, source, fetched_at, &waukesha_entity(None))
    }

    struct Fixture {
        live_page_size: i64,
        historic_page_size: i64,
        live_rows: Vec<RawParcel>,
        historic_rows: Vec<RawParcel>,
        value_rows: Vec<RawParcel>,
        fail_pages: bool,
        count_overrides: Vec<(String, String, i64)>,
        distinct_override: Option<BTreeSet<Option<String>>>,
    }

    impl Fixture {
        fn rows_for(&self, layer_url: &str) -> &[RawParcel] {
            if layer_url == WAUKESHA_HISTORIC {
                &self.historic_rows
            } else if layer_url == VALUE_SOURCE_URL {
                &self.value_rows
            } else {
                &self.live_rows
            }
        }

        fn filtered(&self, layer_url: &str, where_clause: &str) -> Vec<RawParcel> {
            self.rows_for(layer_url)
                .iter()
                .filter(|row| row_matches(row, where_clause))
                .cloned()
                .collect()
        }
    }

    impl ParcelFetch for Fixture {
        fn max_record_count(&self, layer_url: &str) -> Result<i64, StoreError> {
            if layer_url == PARCEL_LAYER_URL {
                Ok(self.live_page_size)
            } else {
                Ok(self.historic_page_size)
            }
        }
        fn return_count_only(&self, layer_url: &str, where_clause: &str) -> Result<i64, StoreError> {
            for (layer, clause, count) in &self.count_overrides {
                if layer == layer_url && clause == where_clause {
                    return Ok(*count);
                }
            }
            Ok(self.filtered(layer_url, where_clause).len() as i64)
        }
        fn distinct_prop_class(
            &self,
            layer_url: &str,
            where_clause: &str,
            _prop_class_field: &str,
            _taxkey_field: &str,
        ) -> Result<BTreeSet<Option<String>>, StoreError> {
            if let Some(ref override_set) = self.distinct_override {
                return Ok(override_set.clone());
            }
            let mut set = BTreeSet::new();
            for row in self.filtered(layer_url, where_clause) {
                set.insert(row.prop_class.clone());
            }
            Ok(set)
        }
        fn query_page(
            &self,
            layer_url: &str,
            where_clause: &str,
            offset: i64,
            page_size: i64,
            _field_map: &FieldMap,
            _vintage_field: Option<&str>,
            _lookup_fields: Option<(&str, &str)>,
        ) -> Result<Vec<RawParcel>, StoreError> {
            if self.fail_pages {
                return Err(StoreError::ParcelFetch("forced page failure".to_string()));
            }
            let rows = self.filtered(layer_url, where_clause);
            let start = offset as usize;
            if start >= rows.len() {
                return Ok(Vec::new());
            }
            let end = (start + page_size as usize).min(rows.len());
            Ok(rows[start..end].to_vec())
        }
    }

    fn parse_muni_eq(where_clause: &str) -> Option<String> {
        let marker = "MUNI = '";
        let start = where_clause.find(marker)? + marker.len();
        let rest = &where_clause[start..];
        let end = rest.find('\'')?;
        Some(rest[..end].to_string())
    }

    fn row_matches(row: &RawParcel, where_clause: &str) -> bool {
        let muni_ok = match parse_muni_eq(where_clause) {
            Some(muni) => row.muni_code.as_deref() == Some(muni.as_str()),
            None => true,
        };
        if !muni_ok {
            return false;
        }
        if where_clause.contains("PROPCLASS LIKE '%1%'") {
            row.prop_class
                .as_deref()
                .map(propclass_like_1)
                .unwrap_or(false)
        } else {
            true
        }
    }

    fn sample_row(taxkey: &str, owner: &str, muni: &str, improvement: f64) -> RawParcel {
        sample_row_class(taxkey, owner, muni, improvement, "1", "2026-08-23")
    }

    fn sample_row_class(
        taxkey: &str,
        owner: &str,
        muni: &str,
        improvement: f64,
        prop_class: &str,
        export_date: &str,
    ) -> RawParcel {
        let normalised = normalise_taxkey(taxkey);
        RawParcel {
            taxkey_required_normalisation: taxkey != normalised,
            taxkey: normalised,
            object_id: Some(1),
            muni_code: Some(muni.to_string()),
            lookup_muni_code: Some(muni.to_string()),
            lookup_muni_name: Some(format!("{muni} name")),
            owner_name: Some(owner.to_string()),
            site_address: Some("1 MAIN ST".to_string()),
            mail_address_1: Some("1 MAIN ST".to_string()),
            mail_address_2: Some("WAUKESHA, WI 53188".to_string()),
            assessed_value: Some(improvement + 1000.0),
            improvement_value: Some(improvement),
            land_value: Some(1000.0),
            owner_type: Some(classify_owner_type(owner).to_string()),
            mail_matches_site: Some(1),
            prop_class: Some(prop_class.to_string()),
            rec_source: None,
            value_as_of: Some(export_date.to_string()),
            export_date: Some(export_date.to_string()),
            value_source: None,
            source_layer_url: None,
        }
    }

    fn live_only(live_page_size: i64, live_rows: Vec<RawParcel>) -> Fixture {
        Fixture {
            live_page_size,
            historic_page_size: 10,
            live_rows,
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: Vec::new(),
            distinct_override: None,
        }
    }

    fn open_store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (source_id, status, finished_at) VALUES ('seed', 'success', '2026-01-01')",
            [],
        )
        .unwrap();
        let seed = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
             VALUES ('55133', 'PWC', 'CITY OF PEWAUKEE', 'test', ?1)",
            [seed],
        )
        .unwrap();
        conn
    }

    fn seed_id(conn: &Connection) -> i64 {
        conn.query_row("SELECT MIN(id) FROM load_runs", [], |row| row.get(0))
            .unwrap()
    }

    fn add_lookup(conn: &Connection, code: &str, name: &str) {
        add_lookup_for(conn, "55133", code, name);
    }

    fn add_lookup_for(conn: &Connection, county_fips: &str, code: &str, name: &str) {
        conn.execute(
            "INSERT INTO muni_lookup (county_fips, muni_code, muni_name, source, load_run_id)
             VALUES (?1, ?2, ?3, 'test', ?4)",
            rusqlite::params![county_fips, code, name, seed_id(conn)],
        )
        .unwrap();
    }

    #[test]
    fn like_match_includes_mixed_residential_and_excludes_non_residential() {
        assert!(propclass_like_1("1,4"));
        assert!(propclass_like_1("4,1"));
        assert!(propclass_like_1("1"));
        assert!(!propclass_like_1("2,4"));
    }

    #[test]
    fn paging_assembles_all_pages_and_stops() {
        let mut conn = open_store();
        let rows = vec![
            sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
            sample_row("PWC0002", "BETA LLC", "PWC", 1.0),
            sample_row("PWC0003", "GAMMA LLC", "PWC", 1.0),
            sample_row("PWC0004", "DELTA LLC", "PWC", 1.0),
            sample_row("PWC0005", "EPSILON LLC", "PWC", 1.0),
        ];
        let result = load_parcels(&mut conn, &live_only(2, rows), "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result.page_size, 2);
        assert_eq!(result.pages_fetched, 3);
        assert_eq!(result.records_written, 5);
        let mut keys = conn
            .prepare("SELECT taxkey FROM entities WHERE load_run_id = ?1 ORDER BY taxkey")
            .unwrap();
        let got: Vec<String> = keys
            .query_map([result.load_run_id], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(got, vec!["PWC0001", "PWC0002", "PWC0003", "PWC0004", "PWC0005"]);
    }

    #[test]
    fn taxkey_normalisation_strips_space_and_leaves_clean_key() {
        assert_eq!(normalise_taxkey("PWC 0924983009"), "PWC0924983009");
        assert_eq!(normalise_taxkey("OTWT1694129"), "OTWT1694129");
    }

    #[test]
    fn failed_assertion_rolls_back_and_writes_zero_rows() {
        let mut conn = open_store();
        let rows = vec![
            sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
            sample_row("PWC0002", "BETA LLC", "PWC", 1.0),
        ];
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: rows,
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![(
                PARCEL_LAYER_URL.to_string(),
                "1=1".to_string(),
                3,
            )],
            distinct_override: None,
        };
        let err = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "A"),
            other => panic!("expected assertion A, got {other:?}"),
        }
        let entities: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entities, 0);
        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM load_runs WHERE status = 'failed' AND error_text = 'failed assertion A'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(failed, 1);
        let running: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM load_runs WHERE status = 'running'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(running, 0);
    }

    #[test]
    fn second_load_run_inserts_rather_than_updates() {
        let mut conn = open_store();
        let source = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 10.0)]);
        let first = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        let second = load_parcels(&mut conn, &source, "2026-08-31T01:00:00Z").unwrap();
        assert_ne!(first.load_run_id, second.load_run_id);
        let entity_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entities WHERE taxkey = 'PWC0001'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(entity_rows, 2);
        let success_runs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM load_runs WHERE status = 'success' AND source_id = 'src-waukesha-parcels'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(success_runs, 2);
        let current: i64 = conn
            .query_row("SELECT COUNT(*) FROM current_entities", [], |row| row.get(0))
            .unwrap();
        assert_eq!(current, 1);
    }

    #[test]
    fn fallback_set_is_computed_from_zero_live_counts() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
            historic_rows: vec![sample_row_class(
                "OTWT0001",
                "BETA LLC",
                "OTWT",
                1.0,
                "1",
                "2026-01-30",
            )],
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: Vec::new(),
            distinct_override: None,
        };
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result.fallback_codes, vec!["OTWT".to_string()]);
        assert!(result.uncovered_codes.is_empty());

        let mut conn2 = open_store();
        add_lookup(&conn2, "BRC", "CITY OF BROOKFIELD");
        let source2 = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
            historic_rows: vec![sample_row_class(
                "BRC0001",
                "GAMMA LLC",
                "BRC",
                1.0,
                "1",
                "2026-01-30",
            )],
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: Vec::new(),
            distinct_override: None,
        };
        let result2 = load_parcels(&mut conn2, &source2, "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result2.fallback_codes, vec!["BRC".to_string()]);
        assert_ne!(result.fallback_codes, result2.fallback_codes);
    }

    #[test]
    fn coverage_action_uses_ratio_when_threshold_present() {
        assert_eq!(
            coverage_action(15_693, 34, Some(0.50)),
            CoverageAction::Fallback
        );
        assert_eq!(
            coverage_action(1_921, 1_362, Some(0.50)),
            CoverageAction::Covered
        );
        assert_eq!(
            coverage_action(0, 0, Some(0.50)),
            CoverageAction::Uncovered
        );
        assert_eq!(
            coverage_action(15_693, 34, Some(0.001)),
            CoverageAction::Covered
        );
        assert_eq!(
            coverage_action(1_921, 1_362, Some(0.80)),
            CoverageAction::Fallback
        );
        assert_eq!(
            coverage_action(0, 0, None),
            CoverageAction::Fallback
        );
    }

    #[test]
    fn municipality_at_34_of_15693_falls_back() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
            historic_rows: vec![sample_row_class(
                "OTWT0001",
                "BETA LLC",
                "OTWT",
                1.0,
                "1",
                "2026-01-30",
            )],
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_eq_where("MUNI", "OTWT"),
                    15_693,
                ),
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_classed_where("1=1", "MUNI", "OTWT"),
                    34,
                ),
            ],
            distinct_override: None,
        };
        let result =
            super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &waukesha_entity(Some(0.50))).unwrap();
        assert!(
            result.fallback_codes.iter().any(|code| code == "OTWT"),
            "expected OTWT in fallback, got {:?}",
            result.fallback_codes
        );
        assert!(!result.uncovered_codes.iter().any(|code| code == "OTWT"));
    }

    #[test]
    fn municipality_at_1362_of_1921_does_not_fall_back() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let mut live_rows: Vec<RawParcel> =
            vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)];
        live_rows.extend((0..1362).map(|i| {
            sample_row(&format!("OTWT{i:06}"), "ALPHA LLC", "OTWT", 1.0)
        }));
        live_rows.extend((0..559).map(|i| {
            sample_row_class(
                &format!("OTWTX{i:05}"),
                "BETA LLC",
                "OTWT",
                1.0,
                "2",
                "2026-08-23",
            )
        }));
        let source = Fixture {
            live_page_size: 2000,
            historic_page_size: 10,
            live_rows,
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_eq_where("MUNI", "OTWT"),
                    1_921,
                ),
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_classed_where("1=1", "MUNI", "OTWT"),
                    1_362,
                ),
            ],
            distinct_override: None,
        };
        let result =
            super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &waukesha_entity(Some(0.50))).unwrap();
        assert!(
            !result.fallback_codes.iter().any(|code| code == "OTWT"),
            "expected OTWT not in fallback, got {:?}",
            result.fallback_codes
        );
        assert_eq!(result.records_written, 1_922);
    }

    #[test]
    fn unfiltered_zero_is_uncovered_without_dividing() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let source = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)]);
        let result =
            super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &waukesha_entity(Some(0.50)))
                .unwrap();
        assert!(
            result.uncovered_codes.iter().any(|code| code == "OTWT"),
            "unfiltered zero is uncovered, got {:?}",
            result.uncovered_codes
        );
        assert_eq!(result.records_written, 1);
    }

    #[test]
    fn changing_threshold_changes_fallback_set() {
        let mut conn_low = open_store();
        add_lookup(&conn_low, "OTWT", "TOWN OF OTTAWA");
        let mut conn_high = open_store();
        add_lookup(&conn_high, "OTWT", "TOWN OF OTTAWA");
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                sample_row("OTWT0001", "BETA LLC", "OTWT", 1.0),
                sample_row("OTWT0002", "GAMMA LLC", "OTWT", 1.0),
                sample_row_class("OTWT0003", "DELTA LLC", "OTWT", 1.0, "2", "2026-08-23"),
            ],
            historic_rows: vec![sample_row_class(
                "OTWT0004",
                "EPSILON LLC",
                "OTWT",
                1.0,
                "1",
                "2026-01-30",
            )],
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![(
                PARCEL_LAYER_URL.to_string(),
                muni_classed_where("1=1", "MUNI", "OTWT"),
                2,
            )],
            distinct_override: None,
        };
        let covered =
            super::load_parcels(&mut conn_low, &source, "2026-08-31T00:00:00Z", &waukesha_entity(Some(0.50)))
                .unwrap();
        let fell_back =
            super::load_parcels(&mut conn_high, &source, "2026-08-31T00:00:00Z", &waukesha_entity(Some(0.80)))
                .unwrap();
        assert!(
            !covered.fallback_codes.iter().any(|code| code == "OTWT"),
            "0.50 should leave 2/3 covered, got {:?}",
            covered.fallback_codes
        );
        assert!(
            fell_back.fallback_codes.iter().any(|code| code == "OTWT"),
            "0.80 should fall back 2/3, got {:?}",
            fell_back.fallback_codes
        );
    }

    #[test]
    fn assertion_h_fails_when_written_count_diverges() {
        let mut conn = open_store();
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_eq_where("MUNI", "PWC"),
                    100,
                ),
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_classed_where("1=1", "MUNI", "PWC"),
                    100,
                ),
            ],
            distinct_override: None,
        };
        let err = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &waukesha_entity(Some(0.50)))
            .unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "H"),
            other => panic!("expected assertion H, got {other:?}"),
        }
    }

    #[test]
    fn assertion_h_passes_when_one_of_eighteen_is_skipped() {
        let mut conn = open_store();
        let mut live_rows: Vec<RawParcel> = (1..=17)
            .map(|i| sample_row(&format!("PWC{i:04}"), "ALPHA LLC", "PWC", 1.0))
            .collect();
        let mut gap = placeholder_row("GAP");
        gap.muni_code = Some("PWC".to_string());
        live_rows.push(gap);
        let result = load_parcels(&mut conn, &live_only(20, live_rows), "2026-08-31T00:00:00Z")
            .unwrap();
        assert_eq!(result.records_written, 17);
        assert_eq!(result.records_skipped, 1);
    }

    #[test]
    fn assertion_h_fails_when_two_of_eighteen_are_genuinely_missing() {
        let mut conn = open_store();
        let live_rows: Vec<RawParcel> = (1..=16)
            .map(|i| sample_row(&format!("PWC{i:04}"), "ALPHA LLC", "PWC", 1.0))
            .collect();
        let source = Fixture {
            live_page_size: 20,
            historic_page_size: 10,
            live_rows,
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_eq_where("MUNI", "PWC"),
                    18,
                ),
                (
                    PARCEL_LAYER_URL.to_string(),
                    muni_classed_where("1=1", "MUNI", "PWC"),
                    18,
                ),
            ],
            distinct_override: None,
        };
        let err = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &waukesha_entity(Some(0.50)))
            .unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "H"),
            other => panic!("expected assertion H, got {other:?}"),
        }
    }

    #[test]
    fn pass2_excludes_taxkey_already_in_pass1() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
            historic_rows: vec![
                sample_row_class("PWC0001", "ALPHA LLC", "OTWT", 1.0, "1", "2026-01-30"),
                sample_row_class("OTWT0001", "BETA LLC", "OTWT", 1.0, "1", "2026-01-30"),
            ],
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: Vec::new(),
            distinct_override: None,
        };
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result.pass2_excluded, 1);
        assert_eq!(result.records_written, 2);
        assert_eq!(result.cross_pass_duplicate_taxkeys, 0);
        let mut stmt = conn
            .prepare(
                "SELECT taxkey, value_source FROM entities WHERE load_run_id = ?1 ORDER BY taxkey",
            )
            .unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([result.load_run_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![
                ("OTWT0001".to_string(), "historic_year_end".to_string()),
                ("PWC0001".to_string(), "live".to_string()),
            ]
        );
    }

    #[test]
    fn assertion_b_still_fails_when_duplicate_reaches_assembly() {
        let mut conn = open_store();
        let source = live_only(
            10,
            vec![
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
            ],
        );
        let err = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "B"),
            other => panic!("expected assertion B, got {other:?}"),
        }
    }

    #[test]
    fn e2_passes_when_missing_code_is_uncovered() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let source = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)]);
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        assert!(
            result.uncovered_codes.iter().any(|code| code == "OTWT"),
            "expected OTWT uncovered, got {:?}",
            result.uncovered_codes
        );
        assert_eq!(result.records_written, 1);
    }

    #[test]
    fn e2_fails_when_lookup_code_absent_from_fetch() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![(
                PARCEL_LAYER_URL.to_string(),
                muni_classed_where("1=1", "MUNI", "OTWT"),
                1,
            )],
            distinct_override: None,
        };
        let err = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, actual, .. } => {
                assert_eq!(code, "E2");
                assert!(
                    actual.contains("OTWT"),
                    "actual should name missing codes, got {actual}"
                );
            }
            other => panic!("expected assertion E2, got {other:?}"),
        }
        let error_text: String = conn
            .query_row(
                "SELECT error_text FROM load_runs WHERE status = 'failed' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            error_text.contains("E2") && error_text.contains("OTWT"),
            "error_text should name missing codes, got {error_text}"
        );
        let entities: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entities, 0);
    }

    #[test]
    fn two_vintages_in_one_run_keep_their_value_as_of() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row_class(
                "PWC0001",
                "ALPHA LLC",
                "PWC",
                1.0,
                "1,4",
                "2026-08-23",
            )],
            historic_rows: vec![sample_row_class(
                "OTWT0001",
                "BETA LLC",
                "OTWT",
                1.0,
                "4,1",
                "2026-01-30",
            )],
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: Vec::new(),
            distinct_override: None,
        };
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT taxkey, value_source, value_as_of FROM entities WHERE load_run_id = ?1 ORDER BY taxkey",
            )
            .unwrap();
        let rows: Vec<(String, String, String)> = stmt
            .query_map([result.load_run_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "OTWT0001".to_string(),
                    "historic_year_end".to_string(),
                    "2026-01-30".to_string()
                ),
                (
                    "PWC0001".to_string(),
                    "live".to_string(),
                    "2026-08-23".to_string()
                ),
            ]
        );
        let layer_url: Option<String> = conn
            .query_row(
                "SELECT layer_url FROM load_runs WHERE id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .unwrap();
        let value_as_of: Option<String> = conn
            .query_row(
                "SELECT value_as_of FROM load_runs WHERE id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(layer_url, None);
        assert_eq!(value_as_of, None);
        assert_eq!(result.measured_lag_days, None);
        assert_eq!(result.live_lag_days, Some(8));
        assert_eq!(result.historic_lag_days, Some(213));
    }

    #[test]
    fn finished_at_is_taken_at_commit() {
        let mut conn = open_store();
        let source = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)]);
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        let (started, finished): (String, String) = conn
            .query_row(
                "SELECT started_at, finished_at FROM load_runs WHERE id = ?1",
                [result.load_run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(started, "2026-08-31T00:00:00Z");
        assert_ne!(started, finished);
    }

    #[test]
    fn unfiltered_where_keeps_non_residential_prop_class() {
        let mut conn = open_store();
        let mut commercial = sample_row("PWC0002", "WAREHOUSE LLC", "PWC", 1.0);
        commercial.prop_class = Some("2".to_string());
        let mut vacant = sample_row("PWC0003", "VACANT LLC", "PWC", 0.0);
        vacant.prop_class = None;
        let source = live_only(
            10,
            vec![
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                commercial,
                vacant,
            ],
        );
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result.records_written, 3);
        let mut stmt = conn
            .prepare(
                "SELECT taxkey, prop_class FROM entities WHERE load_run_id = ?1 ORDER BY taxkey",
            )
            .unwrap();
        let rows: Vec<(String, Option<String>)> = stmt
            .query_map([result.load_run_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![
                ("PWC0001".to_string(), Some("1".to_string())),
                ("PWC0002".to_string(), Some("2".to_string())),
                ("PWC0003".to_string(), None),
            ]
        );
    }

    fn placeholder_row(taxkey: &str) -> RawParcel {
        let mut row = sample_row(taxkey, "PLACEHOLDER", "PWC", 0.0);
        row.muni_code = None;
        row.owner_name = None;
        row.improvement_value = None;
        row.value_as_of = None;
        row.export_date = None;
        row
    }

    #[test]
    fn skip_rule_splits_known_placeholder_and_unknown_non_key() {
        let (kept, known, unknown, skipped_by_muni) = skip_non_parcel_keys(vec![
            sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
            placeholder_row("GAP"),
            placeholder_row("TL"),
        ]);
        assert_eq!(
            kept.iter()
                .map(|row| row.taxkey.as_str())
                .collect::<Vec<_>>(),
            vec!["PWC0001"]
        );
        assert_eq!(known.get("GAP"), Some(&1));
        assert_eq!(unknown.get("TL"), Some(&1));
        assert_eq!(skipped_by_muni.get("<null>"), Some(&2));
    }

    #[test]
    fn named_placeholders_are_skipped_and_counted() {
        let mut conn = open_store();
        let source = live_only(
            10,
            vec![
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                placeholder_row("GAP"),
                placeholder_row("Gap"),
                placeholder_row("OVERLAP"),
                placeholder_row("WAKCPending"),
                placeholder_row("COMMON"),
                placeholder_row("CITY"),
            ],
        );
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result.records_written, 1);
        assert_eq!(result.records_skipped, 6);
        let skipped: i64 = conn
            .query_row(
                "SELECT records_skipped FROM load_runs WHERE id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(skipped, 6);
        let error_text: Option<String> = conn
            .query_row(
                "SELECT error_text FROM load_runs WHERE id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(error_text, None);
        let written: Vec<String> = conn
            .prepare("SELECT taxkey FROM entities WHERE load_run_id = ?1")
            .unwrap()
            .query_map([result.load_run_id], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(written, vec!["PWC0001".to_string()]);
    }

    #[test]
    fn unknown_non_key_is_skipped_counted_and_reported() {
        let mut conn = open_store();
        let source = live_only(
            10,
            vec![
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                placeholder_row("VOID"),
            ],
        );
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result.records_written, 1);
        assert_eq!(result.records_skipped, 1);
        let written: Vec<String> = conn
            .prepare("SELECT taxkey FROM entities WHERE load_run_id = ?1")
            .unwrap()
            .query_map([result.load_run_id], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(written, vec!["PWC0001".to_string()]);
        let error_text: String = conn
            .query_row(
                "SELECT error_text FROM load_runs WHERE id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            error_text.starts_with("skipped unknown non-keys: ") && error_text.contains("VOID=1"),
            "got {error_text}"
        );
    }

    #[test]
    fn taxkey_repeating_more_than_twice_fails_assertion_b() {
        let mut conn = open_store();
        let source = live_only(
            10,
            vec![
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
            ],
        );
        let err = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "B"),
            other => panic!("expected assertion B, got {other:?}"),
        }
    }

    #[test]
    fn assertion_k_fails_when_written_plus_skipped_ne_fetched() {
        let mut conn = open_store();
        conn.execute(
            "INSERT INTO load_runs (source_id, status, started_at)
             VALUES ('src-waukesha-parcels', 'running', '2026-08-31T00:00:00Z')",
            [],
        )
        .unwrap();
        let load_run_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, county_fips, taxkey, muni_code, owner_name,
                 improvement_value, owner_type, prop_class, value_source,
                 value_as_of, fetched_at
             ) VALUES (
                 ?1, '55133', 'PWC0001', 'PWC', 'ALPHA LLC',
                 1.0, 'entity', '1', 'live',
                 '2026-08-23', '2026-08-31T00:00:00Z'
             )",
            [load_run_id],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let expected_prop = {
            let mut set = BTreeSet::new();
            set.insert(Some("1".to_string()));
            set
        };
        let err = run_assertions(
            &tx,
            load_run_id,
            1,
            0,
            1,
            0,
            0,
            0,
            0,
            &[],
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &expected_prop,
            0,
            2,
            "55133",
        )
        .unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "K"),
            other => panic!("expected assertion K, got {other:?}"),
        }
    }

    #[test]
    fn owner_and_improvement_nulls_are_stored_and_do_not_fail() {
        let mut conn = open_store();
        let mut row = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        row.owner_name = None;
        row.improvement_value = None;
        row.value_as_of = None;
        row.export_date = None;
        let result = load_parcels(&mut conn, &live_only(10, vec![row]), "2026-08-31T00:00:00Z")
            .unwrap();
        assert_eq!(result.records_written, 1);
        let (owner, imp, as_of): (Option<String>, Option<f64>, Option<String>) = conn
            .query_row(
                "SELECT owner_name, improvement_value, value_as_of FROM entities WHERE load_run_id = ?1",
                [result.load_run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(owner, None);
        assert_eq!(imp, None);
        assert_eq!(as_of, None);
    }

    #[test]
    fn assertion_i_fails_when_distinct_prop_class_diverges() {
        let mut conn = open_store();
        let mut expected = BTreeSet::new();
        expected.insert(Some("1".to_string()));
        expected.insert(Some("2".to_string()));
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: Vec::new(),
            distinct_override: Some(expected),
        };
        let err = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "I"),
            other => panic!("expected assertion I, got {other:?}"),
        }
    }

    #[test]
    fn value_source_fills_null_and_does_not_overwrite() {
        let mut conn = open_store();
        let mut null_values = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        null_values.assessed_value = None;
        null_values.improvement_value = None;
        null_values.land_value = None;
        let mut had_values = sample_row("PWC0002", "BETA LLC", "PWC", 50.0);
        had_values.assessed_value = Some(150.0);
        had_values.improvement_value = Some(50.0);
        had_values.land_value = Some(100.0);
        let mut incoming_null = sample_row("PWC0001", "ALPHA LLC", "PWC", 99.0);
        incoming_null.assessed_value = Some(200.0);
        incoming_null.improvement_value = Some(99.0);
        incoming_null.land_value = Some(80.0);
        let mut incoming_had = sample_row("PWC0002", "BETA LLC", "PWC", 999.0);
        incoming_had.assessed_value = Some(999.0);
        incoming_had.improvement_value = Some(999.0);
        incoming_had.land_value = Some(999.0);
        let extra = sample_row("PWC9999", "GHOST LLC", "PWC", 1.0);
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![null_values, had_values],
            historic_rows: Vec::new(),
            value_rows: vec![incoming_null, incoming_had, extra],
            fail_pages: false,
            count_overrides: Vec::new(),
            distinct_override: None,
        };
        let mut field_map = waukesha_field_map();
        field_map.site_address = None;
        field_map.mail_address_1 = None;
        field_map.mail_address_2 = None;
        field_map.prop_class = Some("PROPCLASS".to_string());
        let entity = Entity {
            spine_source_id: "src-waukesha-parcels".to_string(),
            county_fips: "55133".to_string(),
            natural_key_field: Some("taxkey".to_string()),
            has_stable_key: true,
            spine_coverage_fallback_threshold: None,
            layers: vec![
                waukesha_layer(LayerRole::Primary, PARCEL_LAYER_URL),
                Layer {
                    role: LayerRole::ValueSource,
                    url: VALUE_SOURCE_URL.to_string(),
                    where_clause: "1=1".to_string(),
                    page_size: Some(10),
                    vintage_field: None,
                    field_map,
                },
            ],
            attachments: Vec::new(),
            lookup_source: None,
            collapse_duplicate_geometry: false,
        };
        let result = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .unwrap();
        assert_eq!(result.records_written, 2);
        let mut stmt = conn
            .prepare(
                "SELECT taxkey, assessed_value, improvement_value, land_value
                 FROM entities WHERE load_run_id = ?1 ORDER BY taxkey",
            )
            .unwrap();
        let rows: Vec<(String, Option<f64>, Option<f64>, Option<f64>)> = stmt
            .query_map([result.load_run_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![
                ("PWC0001".to_string(), Some(200.0), Some(99.0), Some(80.0)),
                ("PWC0002".to_string(), Some(150.0), Some(50.0), Some(100.0)),
            ]
        );
    }

    #[test]
    fn classify_owner_type_examples() {
        assert_eq!(classify_owner_type("SANDHILL OF HARTLAND, LLC"), "entity");
        assert_eq!(classify_owner_type("MILLER FAMILY TRUST"), "trust");
        assert_eq!(classify_owner_type("VILLAGE OF MENOMONEE FALLS"), "government");
        assert_eq!(classify_owner_type("KATHERINE ERICKSON"), "individual");
        assert_eq!(
            classify_owner_type("HOME PATH FINANCIAL LIMITED PARTNERSHIP"),
            "entity"
        );
    }

    #[test]
    fn export_date_epoch_ms_to_ymd() {
        assert_eq!(export_date_to_ymd(1787472022000.0).as_deref(), Some("2026-08-23"));
    }

    fn milwaukee_derived_entity() -> Entity {
        Entity {
            spine_source_id: "src-milwaukee-parcels".to_string(),
            county_fips: "55079".to_string(),
            natural_key_field: Some("taxkey".to_string()),
            has_stable_key: true,
            spine_coverage_fallback_threshold: None,
            layers: vec![waukesha_layer(LayerRole::Primary, PARCEL_LAYER_URL)],
            attachments: Vec::new(),
            lookup_source: Some(LookupSource::Derived {
                code_field: "MCD".to_string(),
                name_field: "MUNINAME".to_string(),
            }),
            collapse_duplicate_geometry: false,
        }
    }

    #[test]
    fn lookup_scoped_to_one_county_does_not_see_another() {
        let conn = open_store();
        add_lookup_for(&conn, "55079", "53000", "Milwaukee");
        let codes = lookup_muni_codes(&conn, "55079").unwrap();
        assert_eq!(codes, vec!["53000".to_string()]);
        let waukesha = lookup_muni_codes(&conn, "55133").unwrap();
        assert!(waukesha.contains(&"PWC".to_string()));
        assert!(!waukesha.contains(&"53000".to_string()));
    }

    #[test]
    fn e1_fails_when_fetched_code_exists_only_for_another_county() {
        let mut conn = open_store();
        add_lookup_for(&conn, "55079", "53000", "Milwaukee");
        let mut entity = waukesha_entity(None);
        entity.county_fips = "55079".to_string();
        let source = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)]);
        let err = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, actual, .. } => {
                assert_eq!(code, "E1");
                assert!(
                    actual.contains("unmatched=1") || actual.contains("matched=0"),
                    "got {actual}"
                );
            }
            other => panic!("expected assertion E1, got {other:?}"),
        }
    }

    #[test]
    fn e1_passes_when_fetched_code_exists_for_that_county() {
        let mut conn = open_store();
        let source = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)]);
        let result = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        assert_eq!(result.records_written, 1);
    }

    #[test]
    fn e2_ignores_lookup_codes_belonging_to_other_counties() {
        let mut conn = open_store();
        add_lookup(&conn, "OTWT", "TOWN OF OTTAWA");
        add_lookup_for(&conn, "55079", "53000", "Milwaukee");
        let mut entity = waukesha_entity(None);
        entity.county_fips = "55079".to_string();
        let source = live_only(
            10,
            vec![sample_row("MKE0001", "ALPHA LLC", "53000", 1.0)],
        );
        let result = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .unwrap();
        assert_eq!(result.records_written, 1);
    }

    #[test]
    fn e2_fails_when_this_county_lookup_code_is_missing_from_fetch() {
        let mut conn = open_store();
        add_lookup_for(&conn, "55079", "53000", "Milwaukee");
        add_lookup_for(&conn, "55079", "68325", "River Hills");
        let mut entity = waukesha_entity(None);
        entity.county_fips = "55079".to_string();
        let source = Fixture {
            live_page_size: 10,
            historic_page_size: 10,
            live_rows: vec![sample_row("MKE0001", "ALPHA LLC", "53000", 1.0)],
            historic_rows: Vec::new(),
            value_rows: Vec::new(),
            fail_pages: false,
            count_overrides: vec![(
                PARCEL_LAYER_URL.to_string(),
                muni_classed_where("1=1", "MUNI", "68325"),
                1,
            )],
            distinct_override: None,
        };
        let err = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, actual, .. } => {
                assert_eq!(code, "E2");
                assert!(actual.contains("68325"), "got {actual}");
                assert!(!actual.contains("PWC"), "other-county PWC leaked into E2: {actual}");
            }
            other => panic!("expected assertion E2, got {other:?}"),
        }
    }

    #[test]
    fn derived_lookup_builds_from_fixture_rows_before_assertions() {
        let mut conn = open_store();
        let source = live_only(
            10,
            vec![
                sample_row("MKE0001", "ALPHA LLC", "53000", 1.0),
                sample_row("MKE0002", "BETA LLC", "68325", 1.0),
            ],
        );
        let result = super::load_parcels(
            &mut conn,
            &source,
            "2026-08-31T00:00:00Z",
            &milwaukee_derived_entity(),
        )
        .unwrap();
        assert_eq!(result.records_written, 2);
        let names: Vec<(String, Option<String>)> = conn
            .prepare(
                "SELECT muni_code, muni_name FROM muni_lookup
                 WHERE county_fips = '55079' ORDER BY muni_code",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                ("53000".to_string(), Some("53000 name".to_string())),
                ("68325".to_string(), Some("68325 name".to_string())),
            ]
        );
        let load_run: i64 = conn
            .query_row(
                "SELECT load_run_id FROM muni_lookup WHERE county_fips = '55079' AND muni_code = '53000'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(load_run, result.load_run_id);
    }

    #[test]
    fn no_lookup_source_and_no_rows_for_county_fails_before_assertions() {
        let mut conn = open_store();
        let mut entity = waukesha_entity(None);
        entity.county_fips = "06001".to_string();
        entity.lookup_source = None;
        let source = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)]);
        let err = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .unwrap_err();
        match err {
            StoreError::ParcelFetch(message) => {
                assert!(
                    message.contains("06001") && message.contains("no lookup_source"),
                    "got {message}"
                );
            }
            other => panic!("expected ParcelFetch before assertions, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_geometry_collapses_identical_mapped_fields() {
        let mut conn = open_store();
        let mut a = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        a.object_id = Some(20);
        let mut b = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        b.object_id = Some(10);
        let mut entity = waukesha_entity(None);
        entity.collapse_duplicate_geometry = true;
        let source = live_only(10, vec![a, b]);
        let result = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .unwrap();
        assert_eq!(result.records_written, 1);
        assert_eq!(result.records_skipped, 1);
        let written: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(written, 1);
    }

    #[test]
    fn duplicate_taxkey_differing_mapped_field_fails() {
        let mut conn = open_store();
        let mut a = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        a.object_id = Some(1);
        let mut b = sample_row("PWC0001", "BETA LLC", "PWC", 1.0);
        b.object_id = Some(2);
        let mut entity = waukesha_entity(None);
        entity.collapse_duplicate_geometry = true;
        let source = live_only(10, vec![a, b]);
        let err = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .unwrap_err();
        match err {
            StoreError::ParcelFetch(message) => {
                assert!(
                    message.contains("PWC0001") && message.contains("owner_name"),
                    "got {message}"
                );
            }
            other => panic!("expected ParcelFetch, got {other:?}"),
        }
    }

    #[test]
    fn collapse_duplicate_geometry_off_does_not_skip() {
        let mut conn = open_store();
        let mut a = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        a.object_id = Some(1);
        let mut b = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        b.object_id = Some(2);
        let source = live_only(10, vec![a, b]);
        let err = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "B"),
            other => panic!("expected assertion B, got {other:?}"),
        }
    }

    #[test]
    fn two_rows_same_taxkey_different_muni_persist() {
        let mut conn = open_store();
        let mut a = sample_row("2190027000", "HOUSE OWNER", "84675", 1.0);
        a.object_id = Some(1);
        a.rec_source = Some("MCTREAS".to_string());
        let mut b = sample_row("2190027000", "SHOP OWNER", "53000", 2.0);
        b.object_id = Some(2);
        b.rec_source = Some("MPROP".to_string());
        let mut entity = waukesha_entity(None);
        entity.county_fips = "55079".to_string();
        entity.spine_coverage_fallback_threshold = None;
        entity.layers = vec![waukesha_layer(LayerRole::Primary, PARCEL_LAYER_URL)];
        entity.lookup_source = Some(LookupSource::Derived {
            code_field: "MUNI".to_string(),
            name_field: "MUNINAME".to_string(),
        });
        let source = live_only(10, vec![a, b]);
        let result = super::load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z", &entity)
            .expect("both parcels persist");
        assert_eq!(result.records_written, 2);
        let written: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entities WHERE load_run_id = ?1",
                [result.load_run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(written, 2);
        let rec: Vec<String> = conn
            .prepare("SELECT rec_source FROM entities WHERE load_run_id = ?1 ORDER BY rec_source")
            .unwrap()
            .query_map([result.load_run_id], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(rec, vec!["MCTREAS".to_string(), "MPROP".to_string()]);
    }

    #[test]
    fn two_rows_same_taxkey_same_muni_fail_assertion_b() {
        let mut conn = open_store();
        let mut a = sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0);
        a.object_id = Some(1);
        let mut b = sample_row("PWC0001", "BETA LLC", "PWC", 2.0);
        b.object_id = Some(2);
        let source = live_only(10, vec![a, b]);
        let err = load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap_err();
        match err {
            StoreError::ParcelAssertion { code, .. } => assert_eq!(code, "B"),
            other => panic!("expected assertion B, got {other:?}"),
        }
    }

    #[test]
    fn current_entities_keeps_both_rows_for_colliding_taxkey() {
        let conn = open_store();
        conn.execute(
            "INSERT INTO load_runs (source_id, status, finished_at)
             VALUES ('src-milwaukee-parcels', 'success', '2026-09-03T00:00:00Z')",
            [],
        )
        .unwrap();
        let run = conn.last_insert_rowid();
        for muni in ["84675", "53000"] {
            conn.execute(
                "INSERT INTO entities (load_run_id, county_fips, taxkey, muni_code, owner_name)
                 VALUES (?1, '55079', '2190027000', ?2, 'OWNER')",
                rusqlite::params![run, muni],
            )
            .unwrap();
        }
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_entities WHERE taxkey = '2190027000'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
        let munis: Vec<String> = conn
            .prepare("SELECT muni_code FROM current_entities WHERE taxkey = '2190027000' ORDER BY muni_code")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(munis, vec!["53000".to_string(), "84675".to_string()]);
    }

    #[test]
    fn assertion_k_sums_three_skip_categories() {
        let mut conn = open_store();
        conn.execute(
            "INSERT INTO load_runs (source_id, status, started_at, finished_at)
             VALUES ('src-waukesha-parcels', 'success', '2026-08-31T00:00:00Z', '2026-08-31T00:00:00Z')",
            [],
        )
        .unwrap();
        let load_run_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, county_fips, taxkey, muni_code, owner_name,
                 improvement_value, owner_type, prop_class, value_source,
                 value_as_of, fetched_at
             ) VALUES (
                 ?1, '55133', 'PWC0001', 'PWC', 'ALPHA LLC',
                 1.0, 'entity', '1', 'live',
                 '2026-08-23', '2026-08-31T00:00:00Z'
             )",
            [load_run_id],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let expected_prop = {
            let mut set = BTreeSet::new();
            set.insert(Some("1".to_string()));
            set
        };
        run_assertions(
            &tx,
            load_run_id,
            5,
            0,
            1,
            4,
            2,
            1,
            1,
            &[],
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &expected_prop,
            0,
            5,
            "55133",
        )
        .expect("K should pass when written+2+1+1=5");
    }

    #[test]
    fn fetch_populates_current_parcels_for_fetched_county_only() {
        let mut conn = open_store();
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (50, 'src-milwaukee-parcels', 'success', '2026-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entities (id, load_run_id, county_fips, muni_code, taxkey, owner_name, value_as_of)
             VALUES (50, 50, '55079', '53000', 'MKE1', 'MILW LLC', '2026-01-01')",
            [],
        )
        .unwrap();
        crate::current::rebuild_current_parcels(&conn, None).unwrap();
        let source = live_only(
            10,
            vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)],
        );
        load_parcels(&mut conn, &source, "2026-08-31T00:00:00Z").unwrap();
        let waukesha: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_parcels WHERE county_fips = '55133'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(waukesha, 1);
        let mke: String = conn
            .query_row(
                "SELECT owner_name FROM current_parcels WHERE taxkey = 'MKE1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mke, "MILW LLC");
    }

    #[test]
    fn absent_key_keeps_prior_current_row_and_value_as_of() {
        let mut conn = open_store();
        let first = live_only(
            10,
            vec![
                sample_row_class("PWC0001", "ALPHA LLC", "PWC", 1.0, "1", "2026-01-01"),
                sample_row_class("PWC0002", "BETA LLC", "PWC", 1.0, "1", "2026-01-01"),
            ],
        );
        load_parcels(&mut conn, &first, "2026-08-31T00:00:00Z").unwrap();
        let second = live_only(
            10,
            vec![sample_row_class(
                "PWC0001",
                "ALPHA LLC",
                "PWC",
                2.0,
                "1",
                "2026-02-01",
            )],
        );
        load_parcels(&mut conn, &second, "2026-09-01T00:00:00Z").unwrap();
        let kept: String = conn
            .query_row(
                "SELECT value_as_of FROM current_parcels WHERE taxkey = 'PWC0002'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, "2026-01-01");
        let updated: String = conn
            .query_row(
                "SELECT value_as_of FROM current_parcels WHERE taxkey = 'PWC0001'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(updated, "2026-02-01");
    }

    #[test]
    fn failed_assertion_leaves_current_parcels_unchanged() {
        let mut conn = open_store();
        let ok = live_only(10, vec![sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0)]);
        load_parcels(&mut conn, &ok, "2026-08-31T00:00:00Z").unwrap();
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM current_parcels", [], |row| row.get(0))
            .unwrap();
        let bad = live_only(
            10,
            vec![
                sample_row("PWC0001", "ALPHA LLC", "PWC", 1.0),
                sample_row("PWC0002", "BETA LLC", "PWC", 1.0),
            ],
        );
        let source = Fixture {
            count_overrides: vec![(PARCEL_LAYER_URL.to_string(), "1=1".to_string(), 3)],
            ..bad
        };
        assert!(load_parcels(&mut conn, &source, "2026-09-01T00:00:00Z").is_err());
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM current_parcels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, after);
        let name: String = conn
            .query_row(
                "SELECT owner_name FROM current_parcels WHERE taxkey = 'PWC0001'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "ALPHA LLC");
    }
}
