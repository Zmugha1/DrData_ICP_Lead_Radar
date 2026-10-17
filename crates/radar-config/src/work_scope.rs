use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub key: String,
    pub label: String,
    pub ticket_low: u32,
    pub ticket_high: u32,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capacity {
    pub builds_concurrent: u32,
    pub first_meetings_per_week: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkScope {
    pub jobs: Vec<Job>,
    pub capacity: Capacity,
}
