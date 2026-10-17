use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchQueryCategory {
    pub key: String,
    pub vertical_keys: Vec<String>,
    pub queries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchQueryTemplates {
    pub categories: Vec<SearchQueryCategory>,
}
