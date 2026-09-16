use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CoverageLevel {
    Full,
    Partial,
    None,
}

impl CoverageLevel {
    pub fn as_label(&self) -> &'static str {
        match self {
            CoverageLevel::Full => "full",
            CoverageLevel::Partial => "partial",
            CoverageLevel::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEntry {
    pub key: String,
    pub level: CoverageLevel,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationLag {
    pub source_id: String,
    pub lag_days: u32,
    pub measured: bool,
    pub measured_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub geographies: Vec<CoverageEntry>,
    pub attributes: Vec<CoverageEntry>,
    pub publication_lags: Vec<PublicationLag>,
}
