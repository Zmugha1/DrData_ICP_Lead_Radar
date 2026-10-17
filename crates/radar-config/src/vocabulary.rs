use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerticalVocabulary {
    pub vertical_key: String,
    #[serde(default)]
    pub classifier_positive: Vec<String>,
    #[serde(default)]
    pub classifier_negative: Vec<String>,
    #[serde(default)]
    pub outreach_lead: Vec<String>,
    #[serde(default)]
    pub outreach_avoid: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vocabulary {
    #[serde(default)]
    pub verticals: Vec<VerticalVocabulary>,
}

impl Vocabulary {
    pub fn for_vertical(&self, vertical_key: &str) -> Option<&VerticalVocabulary> {
        self.verticals
            .iter()
            .find(|entry| entry.vertical_key == vertical_key)
    }

    pub fn from_test_lists(
        classifier_positive: Vec<String>,
        classifier_negative: Vec<String>,
        outreach_lead: Vec<String>,
        outreach_avoid: Vec<String>,
    ) -> Self {
        Self {
            verticals: vec![VerticalVocabulary {
                vertical_key: "test".to_string(),
                classifier_positive,
                classifier_negative,
                outreach_lead,
                outreach_avoid,
            }],
        }
    }
}
