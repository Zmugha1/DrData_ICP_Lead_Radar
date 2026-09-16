use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub key: String,
    pub source_id: String,
    pub published: bool,
    pub wired: bool,
    pub lag_days: Option<u32>,
    pub hard_stop: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Triggers {
    pub triggers: Vec<Trigger>,
}
