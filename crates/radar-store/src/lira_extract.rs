use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::anthropic::{encode_base64, AnthropicClient};
use crate::error::StoreError;
use crate::lira_pdf::{application_id_from_url, list_lira_pdfs};

pub const LIRA_PDF_EXTRACT_SOURCE_ID: &str = "src-milwaukee-lira-pdf-extraction";
pub const EXTRACTION_TOOL_NAME: &str = "record_license_application_fields";

pub const EXTRACTION_SCHEMA_FIELDS: &[&str] = &[
    "trade_name",
    "legal_entity_name",
    "business_address",
    "business_phone",
    "contact_email",
    "email_is_free_provider",
    "email_domain",
    "license_type",
    "service_style",
    "sells_regulated_goods",
    "sound_amplification",
    "sound_amplification_desc",
    "security_cameras",
    "security_camera_count",
    "security_camera_location",
    "building_owner",
    "building_owner_phone",
    "applicant_is_owner",
];

const TRI_STATE: &[&str] = &["yes", "no", "unknown"];
const SERVICE_STYLES: &[&str] = &[
    "full_service",
    "banquet",
    "caterer",
    "retail",
    "tavern",
    "other",
    "unknown",
];
const FREE_EMAIL_DOMAINS: &[&str] = &[
    "gmail.com",
    "yahoo.com",
    "outlook.com",
    "hotmail.com",
    "aol.com",
];

const SCHEMA_INSTRUCTIONS: &str = r#"Extract business-level fields from this scanned license application.
Return values through the record_license_application_fields tool only.

Schema:
- trade_name: string or null
- legal_entity_name: string or null
- business_address: string or null
- business_phone: business premises or business office phone only, or null
- contact_email: business contact email or null
- license_type: string or null
- service_style: full_service, banquet, caterer, retail, tavern, other, or unknown
- sells_regulated_goods: true if alcohol or tobacco appears on the license, false if the form shows they are not sold, null if unreadable
- sound_amplification: yes, no, or unknown (Plan of Operation section 3e when present)
- sound_amplification_desc: string or null
- security_cameras: yes, no, or unknown (sections 5d to 5e when present)
- security_camera_count: integer or null. Unreadable count is null, never 0
- security_camera_location: string or null
- building_owner: string or null (sections 9g to 9h when present)
- building_owner_phone: landlord or building-owner contact for the premises, or null. Not an applicant home phone
- applicant_is_owner: yes, no, or unknown
- page: object mapping each field name above to the 1-based page number the value was read from, or null
- confidence: object mapping each field name above to a number from 0 to 1
- fields_unreadable: array of field names that could not be read

Rules:
- A field that cannot be read is null and its name is in fields_unreadable. Never guess. Never default.
- An ambiguous checkbox is unknown, not no.
- A form section absent for this license type is unknown, not no.
- Do not extract date of birth, driver license number, social security number, home phone, or any other personal identifier of the applicant or agent.
- Do not extract an owner or agent personal phone under any field name.
- email_domain and email_is_free_provider are omitted; they are derived later from contact_email.
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct FieldValue {
    pub name: String,
    pub text: Option<String>,
    pub page: Option<i64>,
    pub confidence: Option<f64>,
    pub unreadable: bool,
    pub judgment: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionRecord {
    pub application_id: String,
    pub application_source_file: String,
    pub source_url: Option<String>,
    pub model_id: String,
    pub extracted_at: String,
    pub trade_name: Option<String>,
    pub legal_entity_name: Option<String>,
    pub business_address: Option<String>,
    pub business_phone: Option<String>,
    pub contact_email: Option<String>,
    pub email_is_free_provider: Option<bool>,
    pub email_domain: Option<String>,
    pub license_type: Option<String>,
    pub service_style: Option<String>,
    pub sells_regulated_goods: Option<bool>,
    pub sound_amplification: Option<String>,
    pub sound_amplification_desc: Option<String>,
    pub security_cameras: Option<String>,
    pub security_camera_count: Option<i64>,
    pub security_camera_location: Option<String>,
    pub building_owner: Option<String>,
    pub building_owner_phone: Option<String>,
    pub applicant_is_owner: Option<String>,
    pub fields_unreadable: Vec<String>,
    pub fields: Vec<FieldValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldStats {
    pub field: String,
    pub fill_rate: f64,
    pub unreadable_rate: f64,
    pub judgment_count: i64,
    pub judgment_notes: Vec<String>,
}

pub fn extraction_tool_schema() -> Value {
    let string_or_null = json!({ "type": ["string", "null"] });
    let number_or_null = json!({ "type": ["number", "null"] });
    let integer_or_null = json!({ "type": ["integer", "null"] });
    let bool_or_null = json!({ "type": ["boolean", "null"] });
    json!({
        "name": EXTRACTION_TOOL_NAME,
        "description": "Record business-level fields read from the scanned application images.",
        "input_schema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "trade_name": string_or_null,
                "legal_entity_name": string_or_null,
                "business_address": string_or_null,
                "business_phone": string_or_null,
                "contact_email": string_or_null,
                "license_type": string_or_null,
                "service_style": {
                    "type": ["string", "null"],
                    "enum": ["full_service", "banquet", "caterer", "retail", "tavern", "other", "unknown", null]
                },
                "sells_regulated_goods": bool_or_null,
                "sound_amplification": {
                    "type": ["string", "null"],
                    "enum": ["yes", "no", "unknown", null]
                },
                "sound_amplification_desc": string_or_null,
                "security_cameras": {
                    "type": ["string", "null"],
                    "enum": ["yes", "no", "unknown", null]
                },
                "security_camera_count": integer_or_null,
                "security_camera_location": string_or_null,
                "building_owner": string_or_null,
                "building_owner_phone": string_or_null,
                "applicant_is_owner": {
                    "type": ["string", "null"],
                    "enum": ["yes", "no", "unknown", null]
                },
                "page": {
                    "type": "object",
                    "additionalProperties": number_or_null
                },
                "confidence": {
                    "type": "object",
                    "additionalProperties": number_or_null
                },
                "fields_unreadable": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["fields_unreadable"]
        }
    })
}

pub fn extraction_messages(page_jpegs: &[(i64, Vec<u8>)]) -> Value {
    let mut content = vec![json!({
        "type": "text",
        "text": SCHEMA_INSTRUCTIONS
    })];
    for (page, bytes) in page_jpegs {
        content.push(json!({
            "type": "text",
            "text": format!("Page {page} of the application packet.")
        }));
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": encode_base64(bytes)
            }
        }));
    }
    json!([{ "role": "user", "content": content }])
}

pub fn prompt_contains_forbidden_client_text(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("brew city") || lower.contains("dr data") || lower.contains("drdata")
}

pub fn email_domain(email: &str) -> Option<String> {
    let trimmed = email.trim();
    let at = trimmed.rfind('@')?;
    let domain = trimmed[at + 1..].trim().to_ascii_lowercase();
    if domain.is_empty() || !domain.contains('.') {
        None
    } else {
        Some(domain)
    }
}

pub fn email_is_free_provider(domain: &str) -> bool {
    FREE_EMAIL_DOMAINS
        .iter()
        .any(|item| domain.eq_ignore_ascii_case(item))
}

fn sqlite_now(conn: &Connection) -> Result<String, StoreError> {
    conn.query_row("SELECT datetime('now')", [], |row| row.get(0))
        .map_err(StoreError::from)
}

pub fn rasterize_pdf(pdf: &Path, script: &Path) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let out_dir = std::env::temp_dir().join(format!("radar_lira_pages_{nanos}"));
    fs::create_dir_all(&out_dir).map_err(|err| {
        StoreError::LiraPdf(format!("create {}: {err}", out_dir.display()))
    })?;
    let output = Command::new("python")
        .arg(script)
        .arg(pdf)
        .arg(&out_dir)
        .output()
        .map_err(|err| StoreError::LiraPdf(format!("run rasterizer: {err}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_dir_all(&out_dir);
        return Err(StoreError::LiraPdf(format!(
            "rasterizer failed: {stderr}"
        )));
    }
    let mut pages = Vec::new();
    let mut files: Vec<PathBuf> = fs::read_dir(&out_dir)
        .map_err(|err| StoreError::LiraPdf(err.to_string()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jpg"))
        .collect();
    files.sort();
    for (index, path) in files.iter().enumerate() {
        let bytes = fs::read(path).map_err(|err| StoreError::LiraPdf(err.to_string()))?;
        pages.push(((index as i64) + 1, bytes));
    }
    let _ = fs::remove_dir_all(&out_dir);
    if pages.is_empty() {
        return Err(StoreError::LiraPdf(format!(
            "rasterizer produced no pages for {}",
            pdf.display()
        )));
    }
    Ok(pages)
}

fn json_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        _ => None,
    }
}

fn json_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(v) => Some(*v),
        _ => None,
    }
}

fn json_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

fn json_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn tool_input_from_response(response: &Value) -> Result<Value, StoreError> {
    if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
        for block in content {
            let kind = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "tool_use" {
                if let Some(input) = block.get("input") {
                    return Ok(input.clone());
                }
            }
            if kind == "text" {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                        return Ok(parsed);
                    }
                }
            }
        }
    }
    Err(StoreError::Anthropic(
        "response contained no tool_use input".to_string(),
    ))
}

fn map_lookup(map: Option<&Value>, field: &str) -> Option<Value> {
    map.and_then(|m| m.get(field)).cloned()
}

fn normalize_tri(raw: Option<String>, unreadable: bool) -> Option<String> {
    if unreadable {
        return None;
    }
    match raw.as_deref() {
        None => Some("unknown".to_string()),
        Some(v) if TRI_STATE.contains(&v) => Some(v.to_string()),
        Some(_) => Some("unknown".to_string()),
    }
}

fn normalize_service(raw: Option<String>, unreadable: bool) -> Option<String> {
    if unreadable {
        return None;
    }
    match raw.as_deref() {
        None => Some("unknown".to_string()),
        Some(v) if SERVICE_STYLES.contains(&v) => Some(v.to_string()),
        Some(_) => Some("unknown".to_string()),
    }
}

pub fn record_from_tool_input(
    input: &Value,
    application_id: &str,
    source_file: &str,
    source_url: Option<&str>,
    model_id: &str,
) -> ExtractionRecord {
    let mut unreadable: BTreeSet<String> = input
        .get("fields_unreadable")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let pages = input.get("page");
    let confs = input.get("confidence");

    let mut values: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut bools: BTreeMap<String, Option<bool>> = BTreeMap::new();
    let mut ints: BTreeMap<String, Option<i64>> = BTreeMap::new();

    for field in EXTRACTION_SCHEMA_FIELDS {
        if *field == "email_domain" || *field == "email_is_free_provider" {
            continue;
        }
        let raw = input.get(*field).cloned().unwrap_or(Value::Null);
        if raw.is_null() && !unreadable.contains(*field) {
            // Model omitted a value without marking unreadable. That is
            // unreadable, not a default.
            if matches!(
                *field,
                "sound_amplification"
                    | "security_cameras"
                    | "applicant_is_owner"
                    | "service_style"
            ) {
                // tri-state / enum omitted: treat as unknown only when the
                // model listed the field as unknown. Omission stays unreadable.
            }
            unreadable.insert((*field).to_string());
        }
        match *field {
            "sells_regulated_goods" => {
                bools.insert((*field).to_string(), json_bool(&raw));
            }
            "security_camera_count" => {
                ints.insert((*field).to_string(), json_i64(&raw));
            }
            _ => {
                values.insert((*field).to_string(), json_string(&raw));
            }
        }
    }

    let contact_email = values.get("contact_email").cloned().flatten();
    let (email_domain_val, email_free) = match contact_email.as_deref().and_then(email_domain) {
        Some(domain) => {
            unreadable.remove("contact_email");
            unreadable.remove("email_domain");
            unreadable.remove("email_is_free_provider");
            let free = email_is_free_provider(&domain);
            (Some(domain), Some(free))
        }
        None => {
            if contact_email.is_some() {
                unreadable.insert("contact_email".to_string());
            }
            unreadable.insert("email_domain".to_string());
            unreadable.insert("email_is_free_provider".to_string());
            (None, None)
        }
    };

    let sound = {
        let unread = unreadable.contains("sound_amplification");
        normalize_tri(values.get("sound_amplification").cloned().flatten(), unread)
    };
    if sound.as_deref() == Some("unknown") {
        unreadable.remove("sound_amplification");
    }
    let cameras = {
        let unread = unreadable.contains("security_cameras");
        normalize_tri(values.get("security_cameras").cloned().flatten(), unread)
    };
    if cameras.as_deref() == Some("unknown") {
        unreadable.remove("security_cameras");
    }
    let owner = {
        let unread = unreadable.contains("applicant_is_owner");
        normalize_tri(values.get("applicant_is_owner").cloned().flatten(), unread)
    };
    if owner.as_deref() == Some("unknown") {
        unreadable.remove("applicant_is_owner");
    }
    let service = {
        let unread = unreadable.contains("service_style");
        normalize_service(values.get("service_style").cloned().flatten(), unread)
    };
    if service.as_deref() == Some("unknown") {
        unreadable.remove("service_style");
    }

    if unreadable.contains("security_camera_count") {
        ints.insert("security_camera_count".to_string(), None);
    }

    let mut fields = Vec::new();
    for field in EXTRACTION_SCHEMA_FIELDS {
        let unread = unreadable.contains(*field);
        let page = if *field == "email_domain" || *field == "email_is_free_provider" {
            map_lookup(pages, "contact_email").and_then(|v| json_i64(&v))
        } else {
            map_lookup(pages, field).and_then(|v| json_i64(&v))
        };
        let mut confidence = map_lookup(confs, field)
            .and_then(|v| json_f64(&v))
            .and_then(|c| if (0.0..=1.0).contains(&c) { Some(c) } else { None });
        if (*field == "email_domain" || *field == "email_is_free_provider") && email_domain_val.is_some()
        {
            confidence = Some(1.0);
        }
        let text = match *field {
            "email_domain" => email_domain_val.clone(),
            "email_is_free_provider" => email_free.map(|v| v.to_string()),
            "sells_regulated_goods" => bools
                .get(*field)
                .cloned()
                .flatten()
                .map(|v| v.to_string()),
            "security_camera_count" => ints
                .get(*field)
                .cloned()
                .flatten()
                .map(|v| v.to_string()),
            "sound_amplification" => sound.clone(),
            "security_cameras" => cameras.clone(),
            "applicant_is_owner" => owner.clone(),
            "service_style" => service.clone(),
            _ => values.get(*field).cloned().flatten(),
        };
        let text = if unread { None } else { text };
        let derived_email = (*field == "email_domain" || *field == "email_is_free_provider")
            && email_domain_val.is_some()
            && !unread;
        let judgment = if derived_email {
            false
        } else {
            unread
                || matches!(text.as_deref(), Some("unknown") | Some("other"))
                || confidence.map(|c| c < 0.85).unwrap_or(text.is_some())
        };
        fields.push(FieldValue {
            name: (*field).to_string(),
            text,
            page,
            confidence,
            unreadable: unread,
            judgment,
        });
    }

    let get = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .and_then(|f| f.text.clone())
    };
    let get_bool = |name: &str| match get(name).as_deref() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };
    let get_i64 = |name: &str| get(name).and_then(|s| s.parse().ok());

    ExtractionRecord {
        application_id: application_id.to_string(),
        application_source_file: source_file.to_string(),
        source_url: source_url.map(|s| s.to_string()),
        model_id: model_id.to_string(),
        extracted_at: String::new(),
        trade_name: get("trade_name"),
        legal_entity_name: get("legal_entity_name"),
        business_address: get("business_address"),
        business_phone: get("business_phone"),
        contact_email: get("contact_email"),
        email_is_free_provider: email_free.filter(|_| !unreadable.contains("email_is_free_provider")),
        email_domain: email_domain_val.filter(|_| !unreadable.contains("email_domain")),
        license_type: get("license_type"),
        service_style: get("service_style"),
        sells_regulated_goods: get_bool("sells_regulated_goods"),
        sound_amplification: get("sound_amplification"),
        sound_amplification_desc: get("sound_amplification_desc"),
        security_cameras: get("security_cameras"),
        security_camera_count: get_i64("security_camera_count"),
        security_camera_location: get("security_camera_location"),
        building_owner: get("building_owner"),
        building_owner_phone: get("building_owner_phone"),
        applicant_is_owner: get("applicant_is_owner"),
        fields_unreadable: unreadable.into_iter().collect(),
        fields,
    }
}

pub fn insert_extraction(
    conn: &mut Connection,
    record: &mut ExtractionRecord,
) -> Result<i64, StoreError> {
    let extracted_at = sqlite_now(conn)?;
    record.extracted_at = extracted_at.clone();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO load_runs (source_id, started_at, finished_at, status, records_requested, records_written)
         VALUES (?1, datetime('now'), datetime('now'), 'success', 1, 1)",
        params![LIRA_PDF_EXTRACT_SOURCE_ID],
    )?;
    let load_run_id = tx.last_insert_rowid();
    let unread_json = serde_json::to_string(&record.fields_unreadable)
        .map_err(|err| StoreError::LiraPdf(err.to_string()))?;
    tx.execute(
        "INSERT INTO lira_pdf_extractions (
            load_run_id, application_id, application_source_file, source_url,
            model_id, extracted_at, trade_name, legal_entity_name, business_address,
            business_phone, contact_email, email_is_free_provider, email_domain,
            license_type, service_style, sells_regulated_goods, sound_amplification,
            sound_amplification_desc, security_cameras, security_camera_count,
            security_camera_location, building_owner, building_owner_phone,
            applicant_is_owner, fields_unreadable
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
            ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
         )",
        params![
            load_run_id,
            record.application_id,
            record.application_source_file,
            record.source_url,
            record.model_id,
            record.extracted_at,
            record.trade_name,
            record.legal_entity_name,
            record.business_address,
            record.business_phone,
            record.contact_email,
            record.email_is_free_provider.map(|v| if v { 1i64 } else { 0 }),
            record.email_domain,
            record.license_type,
            record.service_style,
            record.sells_regulated_goods.map(|v| if v { 1i64 } else { 0 }),
            record.sound_amplification,
            record.sound_amplification_desc,
            record.security_cameras,
            record.security_camera_count,
            record.security_camera_location,
            record.building_owner,
            record.building_owner_phone,
            record.applicant_is_owner,
            unread_json,
        ],
    )?;
    let extraction_id = tx.last_insert_rowid();
    for field in &record.fields {
        tx.execute(
            "INSERT INTO lira_pdf_extraction_fields (
                extraction_id, field_name, value_text, page_number, confidence, unreadable
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                extraction_id,
                field.name,
                field.text,
                field.page,
                field.confidence,
                if field.unreadable { 1i64 } else { 0 }
            ],
        )?;
    }
    tx.commit()?;
    Ok(extraction_id)
}

pub fn field_stats(records: &[ExtractionRecord]) -> Vec<FieldStats> {
    let n = records.len() as f64;
    if n == 0.0 {
        return EXTRACTION_SCHEMA_FIELDS
            .iter()
            .map(|field| FieldStats {
                field: (*field).to_string(),
                fill_rate: 0.0,
                unreadable_rate: 0.0,
                judgment_count: 0,
                judgment_notes: Vec::new(),
            })
            .collect();
    }
    EXTRACTION_SCHEMA_FIELDS
        .iter()
        .map(|field| {
            let mut filled = 0.0;
            let mut unread = 0.0;
            let mut judgment_count = 0;
            let mut judgment_notes = Vec::new();
            for record in records {
                if let Some(fv) = record.fields.iter().find(|f| f.name == *field) {
                    if fv.text.is_some() {
                        filled += 1.0;
                    }
                    if fv.unreadable {
                        unread += 1.0;
                    }
                    if fv.judgment {
                        judgment_count += 1;
                        let why = if fv.unreadable {
                            "unreadable"
                        } else if matches!(fv.text.as_deref(), Some("unknown") | Some("other")) {
                            "enum required a judgment"
                        } else {
                            "confidence missing or below 0.85"
                        };
                        judgment_notes.push(format!(
                            "{} {}: {}",
                            record.application_source_file, field, why
                        ));
                    }
                }
            }
            FieldStats {
                field: (*field).to_string(),
                fill_rate: filled / n,
                unreadable_rate: unread / n,
                judgment_count,
                judgment_notes,
            }
        })
        .collect()
}

pub fn format_validation_report(
    records: &[ExtractionRecord],
    fetch_note: &str,
    hand_notes: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("# License application PDF extraction report\n\n");
    out.push_str("Date: 15 September 2026\n");
    out.push_str("Working directory: `C:\\Users\\zumah\\DrData_ICP_Lead_radar`\n");
    out.push_str("Status: demonstration on a hand-curated set. Not production ready.\n\n");
    out.push_str("See `docs/adr/ADR-0010-lira-pdf-vision-extraction.md` for why vision was used, why retrieval is a pasted URL list, why personal identifiers are excluded, and why this uses its own tables instead of ADR-0006.\n\n");
    out.push_str("No Cargo crate was added. The Anthropic Messages API is called with the existing `ureq` client. Page images are rasterized with `scripts/rasterize_pdf.py` using Python packages already on the machine.\n\n");
    out.push_str("## How to run a demonstration extract\n\n");
    out.push_str("1. Paste one ApplAttachServlet URL per line into `C:\\Users\\zumah\\Desktop\\radar_sources\\milwaukee_lira_pdfs\\urls.txt`.\n");
    out.push_str("2. Set `ANTHROPIC_API_KEY` in the process environment. Optional: `ANTHROPIC_MODEL` (default `claude-sonnet-4-5`).\n");
    out.push_str("3. From this repo: `cargo run -p radar-store --bin fetch_lira_pdfs` then `cargo run -p radar-store --bin extract_lira_pdfs`.\n");
    out.push_str("4. Review this file. Do not run the full application set until the handful is trusted.\n\n");
    out.push_str("The key is never written to a file and never placed in the prompt. The prompt is the page images plus the field schema.\n\n");
    out.push_str("Personal applicant identifiers are not in the schema: date of birth, driver license number, social security number, home phone, and owner or agent personal phone. `building_owner_phone` is the premises landlord contact only.\n\n");
    out.push_str("## Fetch\n\n");
    out.push_str(fetch_note);
    out.push_str("\n\n");
    out.push_str("## Extractions\n\n");
    out.push_str(&format!("record count={}\n\n", records.len()));
    if records.is_empty() {
        out.push_str("No PDFs were extracted. Fill rates below are zero because the denominator is zero applications, not because fields were empty on a form.\n\n");
    }
    for record in records {
        out.push_str(&format!(
            "### {}\n\n",
            record.application_source_file
        ));
        out.push_str(&format!("application_id={}\n", record.application_id));
        out.push_str(&format!("model={}\n", record.model_id));
        out.push_str(&format!("extracted_at={}\n", record.extracted_at));
        out.push_str(&format!(
            "fields_unreadable={}\n\n",
            record.fields_unreadable.join(", ")
        ));
        out.push_str("| field | value | page | confidence | unreadable |\n");
        out.push_str("|---|---|---|---|---|\n");
        for field in &record.fields {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                field.name,
                field.text.as_deref().unwrap_or("null"),
                field
                    .page
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "null".to_string()),
                field
                    .confidence
                    .map(|c| format!("{c:.2}"))
                    .unwrap_or_else(|| "null".to_string()),
                field.unreadable
            ));
        }
        out.push('\n');
    }
    out.push_str("## Per-field rates\n\n");
    out.push_str("| field | fill rate | unreadable rate | judgment count |\n");
    out.push_str("|---|---|---|---|\n");
    for stat in field_stats(records) {
        out.push_str(&format!(
            "| {} | {:.2} | {:.2} | {} |\n",
            stat.field, stat.fill_rate, stat.unreadable_rate, stat.judgment_count
        ));
    }
    out.push_str("\n## Judgment calls\n\n");
    let mut any = false;
    for stat in field_stats(records) {
        for note in stat.judgment_notes {
            any = true;
            out.push_str(&format!("- {note}\n"));
        }
    }
    if !any {
        out.push_str("None. No extracted values were present to judge.\n");
    }
    out.push_str("\n## Hand comparison against the PDF\n\n");
    if hand_notes.is_empty() {
        out.push_str("No application PDF was available on disk for a side-by-side check.\n");
    } else {
        for note in hand_notes {
            out.push_str(note);
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

pub fn extract_pdf(
    client: &AnthropicClient,
    model_id: &str,
    pdf: &Path,
    raster_script: &Path,
    source_url: Option<&str>,
) -> Result<ExtractionRecord, StoreError> {
    let pages = rasterize_pdf(pdf, raster_script)?;
    let messages = extraction_messages(&pages);
    let prompt_check = SCHEMA_INSTRUCTIONS;
    if prompt_contains_forbidden_client_text(prompt_check) {
        return Err(StoreError::LiraPdf(
            "extraction prompt contains forbidden client text".to_string(),
        ));
    }
    let body = json!({
        "model": model_id,
        "max_tokens": 4096,
        "tools": [extraction_tool_schema()],
        "tool_choice": { "type": "tool", "name": EXTRACTION_TOOL_NAME },
        "messages": messages
    });
    let response = client.post_messages(&body)?;
    let input = tool_input_from_response(&response)?;
    let file_name = pdf
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("application.pdf");
    let application_id = source_url
        .and_then(application_id_from_url)
        .or_else(|| {
            pdf.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| file_name.to_string());
    Ok(record_from_tool_input(
        &input,
        &application_id,
        file_name,
        source_url,
        model_id,
    ))
}

pub fn extract_folder(
    conn: &mut Connection,
    client: &AnthropicClient,
    model_id: &str,
    folder: &Path,
    raster_script: &Path,
) -> Result<Vec<ExtractionRecord>, StoreError> {
    let pdfs = list_lira_pdfs(folder)?;
    let mut records = Vec::new();
    for pdf in pdfs {
        let mut record = extract_pdf(client, model_id, &pdf, raster_script, None)?;
        insert_extraction(conn, &mut record)?;
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::migrate;

    #[test]
    fn prompt_has_schema_and_no_client() {
        assert!(SCHEMA_INSTRUCTIONS.contains("trade_name"));
        assert!(SCHEMA_INSTRUCTIONS.contains("fields_unreadable"));
        assert!(!prompt_contains_forbidden_client_text(SCHEMA_INSTRUCTIONS));
        assert!(!SCHEMA_INSTRUCTIONS.to_ascii_lowercase().contains("brew"));
    }

    #[test]
    fn free_email_derivation() {
        assert_eq!(email_domain("Owner@Gmail.COM ").as_deref(), Some("gmail.com"));
        assert!(email_is_free_provider("gmail.com"));
        assert!(!email_is_free_provider("belli.com"));
    }

    #[test]
    fn unreadable_stays_null_not_default() {
        let input = json!({
            "fields_unreadable": ["security_camera_count", "trade_name"],
            "security_camera_count": 0,
            "sound_amplification": "unknown",
            "security_cameras": "yes",
            "applicant_is_owner": "no",
            "service_style": "tavern",
            "contact_email": "info@example.com",
            "page": { "security_cameras": 5 },
            "confidence": { "security_cameras": 0.9 }
        });
        let rec = record_from_tool_input(&input, "1", "1.pdf", None, "claude-test");
        assert!(rec.trade_name.is_none());
        assert!(rec.security_camera_count.is_none());
        assert_eq!(rec.sound_amplification.as_deref(), Some("unknown"));
        assert_eq!(rec.email_domain.as_deref(), Some("example.com"));
        assert_eq!(rec.email_is_free_provider, Some(false));
        assert!(rec
            .fields_unreadable
            .iter()
            .any(|f| f == "security_camera_count"));
    }

    #[test]
    fn insert_only_round_trip() {
        let mut conn = Connection::open_in_memory().expect("mem");
        migrate(&mut conn).expect("migrate");
        let input = json!({
            "fields_unreadable": [],
            "trade_name": "Belli's",
            "legal_entity_name": "Belli's Bistro LLC",
            "sound_amplification": "no",
            "security_cameras": "unknown",
            "applicant_is_owner": "unknown",
            "service_style": "unknown",
            "contact_email": "a@yahoo.com"
        });
        let mut rec = record_from_tool_input(&input, "9", "9.pdf", None, "claude-test");
        insert_extraction(&mut conn, &mut rec).expect("insert");
        insert_extraction(&mut conn, &mut rec).expect("second insert");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM lira_pdf_extractions", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 2);
        let denied = conn.execute(
            "UPDATE lira_pdf_extractions SET trade_name = 'x' WHERE application_id = '9'",
            [],
        );
        assert!(denied.is_err());
    }
}
