use std::collections::HashSet;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum PhraseAnchor {
    Anywhere,
    First,
    Final,
}

fn default_anywhere() -> PhraseAnchor {
    PhraseAnchor::Anywhere
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerTypePhrase {
    pub text: String,
    #[serde(default = "default_anywhere")]
    pub anchor: PhraseAnchor,
    #[serde(default)]
    pub not_followed_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerTypeClass {
    pub type_key: String,
    pub phrases: Vec<OwnerTypePhrase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerTypeFile {
    pub precedence: Vec<String>,
    pub exclude_from_groups_default: Vec<String>,
    pub classes: Vec<OwnerTypeClass>,
}

impl OwnerTypeFile {
    pub fn from_dr_data_file() -> Self {
        toml::from_str(include_str!("../../../config/dr_data_icp/owner_type.toml"))
            .expect("bundled owner_type.toml")
    }

    pub fn compiled(&self) -> CompiledOwnerType {
        CompiledOwnerType::from_file(self)
    }

    pub fn classify(&self, name: &str) -> String {
        self.compiled().classify(name)
    }
}

#[derive(Debug, Clone)]
struct CompiledPhrase {
    needle: Vec<String>,
    anchor: PhraseAnchor,
    nfb: HashSet<String>,
}

/// Phrase lists tokenised once. Same answers as [`OwnerTypeFile::classify`].
#[derive(Debug, Clone)]
pub struct CompiledOwnerType {
    classes: Vec<(String, Vec<CompiledPhrase>)>,
}

impl CompiledOwnerType {
    pub fn from_file(file: &OwnerTypeFile) -> Self {
        let mut classes = Vec::new();
        for type_key in &file.precedence {
            let Some(class) = file
                .classes
                .iter()
                .find(|class| &class.type_key == type_key)
            else {
                continue;
            };
            let phrases = class
                .phrases
                .iter()
                .map(|phrase| CompiledPhrase {
                    needle: tokenize(&phrase.text),
                    anchor: phrase.anchor,
                    nfb: follow_exclusions(phrase),
                })
                .collect();
            classes.push((type_key.clone(), phrases));
        }
        Self { classes }
    }

    pub fn classify(&self, name: &str) -> String {
        let tokens = tokenize(name);
        for (type_key, phrases) in &self.classes {
            for phrase in phrases {
                if compiled_phrase_matches(&tokens, phrase) {
                    return type_key.clone();
                }
            }
        }
        "individual".to_string()
    }
}

pub(crate) fn tokenize(name: &str) -> Vec<String> {
    name.to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_string())
        .collect()
}

fn follow_exclusions(phrase: &OwnerTypePhrase) -> HashSet<String> {
    let Some(entries) = phrase.not_followed_by.as_ref() else {
        return HashSet::new();
    };
    entries
        .iter()
        .flat_map(|entry| tokenize(entry))
        .collect()
}

fn compiled_phrase_matches(tokens: &[String], phrase: &CompiledPhrase) -> bool {
    let needle = phrase.needle.as_slice();
    if needle.is_empty() || needle.len() > tokens.len() {
        return false;
    }
    let n = needle.len();
    let allowed = |start: usize| {
        let after = start + n;
        !(after < tokens.len() && phrase.nfb.contains(&tokens[after]))
    };
    match phrase.anchor {
        PhraseAnchor::Anywhere => (0..=tokens.len() - n)
            .any(|i| tokens[i..i + n] == *needle && allowed(i)),
        PhraseAnchor::First => tokens.get(..n) == Some(needle) && allowed(0),
        PhraseAnchor::Final => {
            let start = tokens.len() - n;
            tokens.get(start..) == Some(needle) && allowed(start)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OwnerTypeFile, PhraseAnchor};

    #[test]
    fn placeholder_is_not_individual() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("incomplete taxroll information"),
            "placeholder"
        );
    }

    #[test]
    fn placeholder_precedes_other_types() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("incomplete taxroll information LLC"),
            "placeholder"
        );
    }

    #[test]
    fn placeholder_precedes_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("INCOMPLETE TAXROLL INFORMATION COUNTY"),
            "placeholder"
        );
    }

    #[test]
    fn waukesha_county_is_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("WAUKESHA COUNTY"), "government");
    }

    #[test]
    fn milwaukee_county_parks_is_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("MILWAUKEE COUNTY PARKS"), "government");
    }

    #[test]
    fn milwaukee_county_treasurer_is_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("MILWAUKEE COUNTY TREASURER"), "government");
    }

    #[test]
    fn wisconsin_dept_of_transportation_is_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("WISCONSIN DEPT OF TRANSPORTATION"),
            "government"
        );
    }

    #[test]
    fn cda_of_the_city_of_cudahy_is_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("CDA OF THE CITY OF CUDAHY"), "government");
    }

    #[test]
    fn county_road_llc_is_not_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("4878 COUNTY ROAD C LLC"), "entity");
        assert_ne!(config.classify("4878 COUNTY ROAD C LLC"), "government");
    }

    #[test]
    fn waukesha_county_hwy_dept_is_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        // HWY is not in COUNTY.not_followed_by. Putting it there would
        // exclude this public body (8 parcels as WAUKESHA COUNTY (HWY DEPT)).
        assert_eq!(config.classify("WAUKESHA COUNTY HWY DEPT"), "government");
        assert_eq!(config.classify("WAUKESHA COUNTY (HWY DEPT)"), "government");
    }

    #[test]
    fn city_of_still_classifies_as_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("CITY OF OCONOMOWOC"), "government");
    }

    #[test]
    fn precedence_resolves_two_list_matches() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("CITY OF ACME LLC"), "government");
        assert_eq!(config.classify("MILLER FAMILY TRUST LLC"), "trust");
    }

    #[test]
    fn changing_an_anchor_changes_the_outcome() {
        let mut config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("CDA OF THE CITY OF CUDAHY"), "government");
        {
            let government = config
                .classes
                .iter_mut()
                .find(|class| class.type_key == "government")
                .expect("government class");
            let city_of = government
                .phrases
                .iter_mut()
                .find(|phrase| phrase.text == "CITY OF")
                .expect("CITY OF phrase");
            city_of.anchor = PhraseAnchor::First;
        }
        assert_ne!(config.classify("CDA OF THE CITY OF CUDAHY"), "government");
    }

    #[test]
    fn ymca_of_greater_waukesha_county_is_nonprofit_not_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("YMCA OF GREATER WAUKESHA COUNTY INC"),
            "nonprofit"
        );
        assert_ne!(
            config.classify("YMCA OF GREATER WAUKESHA COUNTY INC"),
            "government"
        );
    }

    #[test]
    fn habitat_for_humanity_of_waukesha_county_is_nonprofit() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("HABITAT FOR HUMANITY OF WAUKESHA COUNTY INC"),
            "nonprofit"
        );
    }

    #[test]
    fn waukesha_county_land_conservancy_is_nonprofit() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("WAUKESHA COUNTY LAND CONSERVANCY INC"),
            "nonprofit"
        );
    }

    #[test]
    fn forest_county_potawatomi_is_tribal() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("FOREST COUNTY POTAWATOMI"), "tribal");
    }

    #[test]
    fn waukesha_county_is_still_government() {
        let config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(config.classify("WAUKESHA COUNTY"), "government");
        assert_eq!(config.classify("MILWAUKEE COUNTY PARKS"), "government");
        assert_eq!(config.classify("CITY OF MILWAUKEE"), "government");
    }

    #[test]
    fn changing_precedence_changes_the_outcome() {
        let mut config = OwnerTypeFile::from_dr_data_file();
        assert_eq!(
            config.classify("YMCA OF GREATER WAUKESHA COUNTY INC"),
            "nonprofit"
        );
        config.precedence = vec![
            "placeholder".to_string(),
            "government".to_string(),
            "nonprofit".to_string(),
            "tribal".to_string(),
            "trust".to_string(),
            "entity".to_string(),
        ];
        assert_eq!(
            config.classify("YMCA OF GREATER WAUKESHA COUNTY INC"),
            "government"
        );
    }

    #[test]
    fn county_hwy_in_not_followed_by_would_exclude_public_body() {
        let mut config = OwnerTypeFile::from_dr_data_file();
        {
            let government = config
                .classes
                .iter_mut()
                .find(|class| class.type_key == "government")
                .expect("government class");
            let county = government
                .phrases
                .iter_mut()
                .find(|phrase| phrase.text == "COUNTY")
                .expect("COUNTY phrase");
            let mut nfb = county.not_followed_by.clone().unwrap_or_default();
            nfb.push("HWY".to_string());
            county.not_followed_by = Some(nfb);
        }
        assert_ne!(
            config.classify("WAUKESHA COUNTY (HWY DEPT)"),
            "government"
        );
    }
}
