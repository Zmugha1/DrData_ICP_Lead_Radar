use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SourceFamily {
    PropertyEvent,
    BusinessFormation,
    LegalPublicRecord,
    OwnedEarned,
}

impl SourceFamily {
    pub fn as_label(&self) -> &'static str {
        match self {
            SourceFamily::PropertyEvent => "PropertyEvent",
            SourceFamily::BusinessFormation => "BusinessFormation",
            SourceFamily::LegalPublicRecord => "LegalPublicRecord",
            SourceFamily::OwnedEarned => "OwnedEarned",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SourceKind {
    ArcGis,
    Csv,
    Api,
    PageDiff,
    ManualUpload,
    AccelaSession,
}

impl SourceKind {
    pub fn as_label(&self) -> &'static str {
        match self {
            SourceKind::ArcGis => "ArcGis",
            SourceKind::Csv => "Csv",
            SourceKind::Api => "Api",
            SourceKind::PageDiff => "PageDiff",
            SourceKind::ManualUpload => "ManualUpload",
            SourceKind::AccelaSession => "AccelaSession",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SourceStatus {
    Live,
    Profiled,
    Scoped,
    Refused,
}

impl SourceStatus {
    pub fn as_label(&self) -> &'static str {
        match self {
            SourceStatus::Live => "Live",
            SourceStatus::Profiled => "Profiled",
            SourceStatus::Scoped => "Scoped",
            SourceStatus::Refused => "Refused",
        }
    }

    pub fn group_rank(&self) -> u8 {
        match self {
            SourceStatus::Live => 0,
            SourceStatus::Profiled => 1,
            SourceStatus::Scoped => 2,
            SourceStatus::Refused => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEntry {
    pub source_id: String,
    pub display_name: String,
    pub family: SourceFamily,
    pub kind: SourceKind,
    pub status: SourceStatus,
    pub cadence_days: Option<u32>,
    pub publication_lag_days: Option<u32>,
    pub lag_measured: bool,
    pub url: Option<String>,
    pub url_verified: bool,
    pub requires_key: bool,
    pub attribution: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcesFile {
    pub sources: Vec<SourceEntry>,
}
