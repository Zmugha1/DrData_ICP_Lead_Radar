use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignmentsFile {
    pub state_values: Vec<String>,
    pub waukesha_cities: Vec<String>,
    pub milwaukee_cities: Vec<String>,
}

impl AssignmentsFile {
    pub fn is_wisconsin_state(&self, value: &str) -> bool {
        let trimmed = value.trim();
        self.state_values
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(trimmed))
    }

    pub fn derive_county(&self, city: &str) -> Option<&'static str> {
        let needle = city.trim();
        if needle.is_empty() {
            return None;
        }
        if self
            .waukesha_cities
            .iter()
            .any(|listed| listed.trim().eq_ignore_ascii_case(needle))
        {
            return Some("waukesha");
        }
        if self
            .milwaukee_cities
            .iter()
            .any(|listed| listed.trim().eq_ignore_ascii_case(needle))
        {
            return Some("milwaukee");
        }
        None
    }

    pub fn from_dr_data_file() -> Self {
        toml::from_str(include_str!("../../../config/dr_data_icp/assignments.toml"))
            .expect("bundled assignments.toml")
    }
}
