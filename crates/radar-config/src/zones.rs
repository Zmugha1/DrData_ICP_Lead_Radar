use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Zone {
    Inside,
    Below,
    Above,
    Oscillating,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneAssignment {
    pub capability_key: String,
    pub zone: Zone,
    pub rationale: String,
    pub assigned_on: String,
    pub expires_on: String,
    pub display_caveat: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zones {
    pub assignments: Vec<ZoneAssignment>,
}
