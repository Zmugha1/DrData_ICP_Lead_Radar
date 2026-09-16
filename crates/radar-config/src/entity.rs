use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum JoinMethod {
    ExactKey,
    NormalisedName,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub source_id: String,
    pub join_field: String,
    pub join_method: JoinMethod,
    pub expected_match_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum LayerRole {
    Primary,
    Fallback,
    ValueSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldMap {
    pub taxkey: String,
    pub muni_code: String,
    pub owner_name: String,
    #[serde(default)]
    pub site_address: Option<String>,
    #[serde(default)]
    pub mail_address_1: Option<String>,
    #[serde(default)]
    pub mail_address_2: Option<String>,
    #[serde(default)]
    pub assessed_value: Option<String>,
    #[serde(default)]
    pub improvement_value: Option<String>,
    #[serde(default)]
    pub land_value: Option<String>,
    #[serde(default)]
    pub prop_class: Option<String>,
    #[serde(default)]
    pub rec_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub role: LayerRole,
    pub url: String,
    pub where_clause: String,
    #[serde(default)]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub vintage_field: Option<String>,
    pub field_map: FieldMap,
}

fn default_lookup_where() -> String {
    "1=1".to_string()
}

/// How municipality codes are populated for this county.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LookupSource {
    /// A separate civil-divisions (or equivalent) FeatureServer.
    Layer {
        url: String,
        #[serde(default = "default_lookup_where")]
        where_clause: String,
        code_field: String,
        name_field: String,
    },
    /// Code and name fields on the primary parcel layer.
    Derived {
        code_field: String,
        name_field: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub spine_source_id: String,
    pub county_fips: String,
    pub natural_key_field: Option<String>,
    pub has_stable_key: bool,
    /// Fraction of a subgroup's rows on the primary layer that satisfy
    /// the spine's filter, below which that subgroup is fetched from a
    /// fallback layer instead. It is not a join match rate.
    pub spine_coverage_fallback_threshold: Option<f64>,
    pub layers: Vec<Layer>,
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub lookup_source: Option<LookupSource>,
    /// When true, keep the lowest OBJECTID per taxkey and skip the rest
    /// as skipped_duplicate_geometry, after verifying mapped fields match.
    /// Default false. Leave implemented for a future county. Milwaukee
    /// turns it off: the 2 September 2026 profile found no stacked
    /// geometry.
    #[serde(default)]
    pub collapse_duplicate_geometry: bool,
}

impl Entity {
    pub fn primary_layer(&self) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.role == LayerRole::Primary)
    }

    pub fn fallback_layer(&self) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.role == LayerRole::Fallback)
    }

    pub fn value_source_layers(&self) -> Vec<&Layer> {
        self.layers
            .iter()
            .filter(|layer| layer.role == LayerRole::ValueSource)
            .collect()
    }
}
