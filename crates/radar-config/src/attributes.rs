use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ValueType {
    Numeric,
    Coded,
    Text,
    Date,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Direction {
    Favourable,
    Unfavourable,
    Band,
    NoneSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Favourability {
    Favourable,
    Neutral,
    Unfavourable,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Band {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub label: String,
    pub favourability: Favourability,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    pub key: String,
    pub source_table: String,
    pub source_column: String,
    pub value_type: ValueType,
    pub direction: Direction,
    pub bands: Option<Vec<Band>>,
    pub variance_tested: bool,
    pub variance_tested_on: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attributes {
    pub attributes: Vec<Attribute>,
}
