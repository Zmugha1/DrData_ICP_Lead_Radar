use serde::Deserialize;

const SCORING_CHECK_COUNT: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    pub tier_key: String,
    pub label: String,
    pub description: String,
    pub what_they_buy: String,
    pub price_band: String,
    #[serde(default)]
    pub note: Option<String>,
    pub scoring_checks: Vec<String>,
    pub scoring_rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TiersFile {
    pub tiers: Vec<Tier>,
}

impl TiersFile {
    pub fn contains_key(&self, key: &str) -> bool {
        self.tiers.iter().any(|tier| tier.tier_key == key)
    }

    pub fn test_fixture() -> Self {
        let checks = vec![
            "A named decision with money attached".to_string(),
            "Their own historical data exists somewhere, even in spreadsheets".to_string(),
            "One person's judgment is doing the work".to_string(),
            "Data they would not want in a cloud tool".to_string(),
            "An active trigger: hiring an estimator, succession underway, a recent cost shock, a compliance deadline".to_string(),
        ];
        let scoring_rule = "Five of five is a Tier 1 build. Three or four is worth a discovery meeting. Under three is Tier 3 or a pass.".to_string();
        Self {
            tiers: ["1", "2", "3"]
                .into_iter()
                .map(|key| Tier {
                    tier_key: key.to_string(),
                    label: format!("Tier {key}"),
                    description: format!("test description for tier {key}"),
                    what_they_buy: format!("test purchase for tier {key}"),
                    price_band: format!("test price band for tier {key}"),
                    note: None,
                    scoring_checks: checks.clone(),
                    scoring_rule: scoring_rule.clone(),
                })
                .collect(),
        }
    }
}

pub const REQUIRED_SCORING_CHECK_COUNT: usize = SCORING_CHECK_COUNT;
