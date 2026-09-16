use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use radar_config::{check_config_dir, load, ConfigDirError, Direction};
use radar_store::{evaluate_attribute, fetch_owner_groups, AttributeEvaluation};
use rusqlite::Connection;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: profile_groups <destination-db> <config-dir>");
        return ExitCode::from(2);
    }
    let db_path = PathBuf::from(&args[1]);
    let config_dir = PathBuf::from(&args[2]);

    println!("config directory={}", config_dir.display());
    if let Err(err) = check_config_dir(&config_dir) {
        match err {
            ConfigDirError::MissingDirectory => {
                eprintln!("config directory does not exist: {}", config_dir.display());
            }
            ConfigDirError::MissingFiles(files) => {
                eprintln!(
                    "config directory {} is missing files: {}",
                    config_dir.display(),
                    files.join(", ")
                );
            }
        }
        return ExitCode::from(1);
    }

    let config = match load(&config_dir) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("failed to load config: {err}");
            return ExitCode::from(1);
        }
    };
    let conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("failed to open database: {err}");
            return ExitCode::from(1);
        }
    };

    let groups = match fetch_owner_groups(&conn, &config.owner_type) {
        Ok(groups) => groups,
        Err(err) => {
            eprintln!("failed to group: {err}");
            return ExitCode::from(1);
        }
    };

    let vacant_key = config
        .attributes
        .attributes
        .iter()
        .find(|attribute| attribute.direction == Direction::Band)
        .map(|attribute| attribute.key.as_str());

    let total_keys = groups.len();
    let entity_groups: Vec<_> = groups
        .iter()
        .filter(|group| group.owner_type.as_deref() == Some("entity") && !group.mixed_owner_type)
        .collect();
    let entity_multi_name = entity_groups
        .iter()
        .filter(|group| group.member_name_count > 1)
        .count();
    let mixed = groups.iter().filter(|group| group.mixed_owner_type).count();

    println!("profile total distinct group_key={total_keys}");
    println!("profile distinct group_key owner_type=entity={}", entity_groups.len());
    println!("profile entity groups member_name_count>1={entity_multi_name}");
    println!("profile mixed_owner_type true={mixed}");

    let mut ranked = entity_groups.clone();
    ranked.sort_by(|a, b| {
        b.total_parcels
            .cmp(&a.total_parcels)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    println!("profile ten largest entity groups by total_parcels");
    for group in ranked.iter().take(10) {
        let band = match vacant_key {
            Some(key) => match evaluate_attribute(&config, key, group) {
                Ok(AttributeEvaluation::Band { label, .. }) => label,
                Ok(AttributeEvaluation::OutOfBands { .. }) => "OutOfBands".to_string(),
                Ok(AttributeEvaluation::Unknown) => "Unknown".to_string(),
                Ok(AttributeEvaluation::Value { .. }) => "Value".to_string(),
                Err(err) => format!("error:{err}"),
            },
            None => "unknown-attribute".to_string(),
        };
        println!(
            "  display_name={:?} member_name_count={} total_parcels={:?} improvement_value_zero_count={:?} band={band}",
            group.display_name,
            group.member_name_count,
            group.total_parcels,
            group.improvement_value_zero_count
        );
    }

    let mut dist: BTreeMap<String, i64> = BTreeMap::new();
    if let Some(key) = vacant_key {
        for group in &entity_groups {
            let bucket = match evaluate_attribute(&config, key, group) {
                Ok(AttributeEvaluation::Band { label, .. }) => label,
                Ok(AttributeEvaluation::OutOfBands { .. }) => "OutOfBands".to_string(),
                Ok(AttributeEvaluation::Unknown) => "Unknown".to_string(),
                Ok(AttributeEvaluation::Value { .. }) => "Value".to_string(),
                Err(_) => "error".to_string(),
            };
            *dist.entry(bucket).or_insert(0) += 1;
        }
    }
    println!("profile entity group band distribution");
    for (label, count) in &dist {
        println!("  {label}={count}");
    }

    ExitCode::SUCCESS
}
