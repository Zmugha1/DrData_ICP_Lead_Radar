use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::assignments::AssignmentsFile;
use crate::attributes::Attributes;
use crate::owner_type::OwnerTypeFile;
use crate::boundaries::Boundaries;
use crate::coverage::Coverage;
use crate::entity::Entity;
use crate::error::ConfigError;
use crate::manifest::Manifest;
use crate::series::SeriesFile;
use crate::sources::SourcesFile;
use crate::triggers::Triggers;
use crate::vocabulary::Vocabulary;
use crate::work_scope::WorkScope;
use crate::zones::Zones;
use crate::RadarConfig;

pub const REQUIRED_CONFIG_FILES: [&str; 13] = [
    "manifest.toml",
    "entity.toml",
    "attributes.toml",
    "triggers.toml",
    "work_scope.toml",
    "vocabulary.toml",
    "zones.toml",
    "boundaries.toml",
    "coverage.toml",
    "sources.toml",
    "series.toml",
    "assignments.toml",
    "owner_type.toml",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigDirError {
    MissingDirectory,
    MissingFiles(Vec<String>),
}

pub fn config_fingerprint(dir: &Path) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for name in REQUIRED_CONFIG_FILES {
        let path = dir.join(name);
        let bytes = fs::read(&path).unwrap_or_default();
        for byte in name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0x1f;
        hash = hash.wrapping_mul(PRIME);
        for byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:016x}")
}

/// Presence check only. Does not parse files or touch the network.
pub fn check_config_dir(dir: &Path) -> Result<(), ConfigDirError> {
    if !dir.is_dir() {
        return Err(ConfigDirError::MissingDirectory);
    }
    let missing: Vec<String> = REQUIRED_CONFIG_FILES
        .iter()
        .filter(|name| !dir.join(name).is_file())
        .map(|name| (*name).to_string())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ConfigDirError::MissingFiles(missing))
    }
}

pub fn load(dir: &Path) -> Result<RadarConfig, ConfigError> {
    let manifest: Manifest = parse_file(dir, "manifest.toml")?;
    let entity: Entity = parse_file(dir, "entity.toml")?;
    let attributes: Attributes = parse_file(dir, "attributes.toml")?;
    let triggers: Triggers = parse_file(dir, "triggers.toml")?;
    let work_scope: WorkScope = parse_file(dir, "work_scope.toml")?;
    let vocabulary: Vocabulary = parse_file(dir, "vocabulary.toml")?;
    let zones: Zones = parse_file(dir, "zones.toml")?;
    let boundaries: Boundaries = parse_file(dir, "boundaries.toml")?;
    let coverage: Coverage = parse_file(dir, "coverage.toml")?;
    let sources: SourcesFile = parse_file(dir, "sources.toml")?;
    let series: SeriesFile = parse_file(dir, "series.toml")?;
    let assignments: AssignmentsFile = parse_file(dir, "assignments.toml")?;
    let owner_type: OwnerTypeFile = parse_file(dir, "owner_type.toml")?;

    Ok(RadarConfig {
        manifest,
        entity,
        attributes,
        triggers,
        work_scope,
        vocabulary,
        zones,
        boundaries,
        coverage,
        sources,
        series,
        assignments,
        owner_type,
    })
}

fn parse_file<T: DeserializeOwned>(dir: &Path, name: &str) -> Result<T, ConfigError> {
    let path = dir.join(name);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Err(ConfigError::FileMissing { path });
        }
        Err(err) => {
            return Err(ConfigError::FileUnreadable { path, source: err });
        }
    };
    toml::from_str(&text).map_err(|err| map_parse_error(path, err))
}

fn map_parse_error(path: std::path::PathBuf, err: toml::de::Error) -> ConfigError {
    let message = err.to_string();
    if let Some(field) = extract_unknown_field(&message) {
        ConfigError::UnknownField { path, field }
    } else {
        ConfigError::ParseFailed { path, message }
    }
}

fn extract_unknown_field(message: &str) -> Option<String> {
    if let Some(field) = extract_between(message, "unknown field `", "`") {
        return Some(field);
    }
    extract_between(message, "unknown field \"", "\"")
}

fn extract_between(message: &str, start_marker: &str, end_marker: &str) -> Option<String> {
    let start = message.find(start_marker)? + start_marker.len();
    let rest = message.get(start..)?;
    let end = rest.find(end_marker)?;
    Some(rest[..end].to_string())
}
