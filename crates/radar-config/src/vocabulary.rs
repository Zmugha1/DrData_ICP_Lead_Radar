use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vocabulary {
    pub classifier_positive: Vec<String>,
    pub classifier_negative: Vec<String>,
    pub outreach_lead: Vec<String>,
    pub outreach_avoid: Vec<String>,
}
