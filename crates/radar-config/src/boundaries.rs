use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum AbsenceRuleKind {
    Drop,
    Flag,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbsenceRule {
    pub item: String,
    pub rule: AbsenceRuleKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundaries {
    pub approval_required_by_default: bool,
    pub absence_rules: Vec<AbsenceRule>,
    pub never_stored: Vec<String>,
    pub cloud_extraction_allowed_for: Vec<String>,
}
