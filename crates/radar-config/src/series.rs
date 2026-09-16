use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesEntry {
    pub series_id: String,
    pub label: String,
    pub geography: String,
    pub included: bool,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesFile {
    pub series: Vec<SeriesEntry>,
}
