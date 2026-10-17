use std::collections::{BTreeMap, BTreeSet};

use radar_config::OwnerTypeFile;
use rusqlite::Connection;

use crate::error::StoreError;
use crate::normalise::owner_group_key;

#[derive(Debug, Clone, PartialEq)]
pub struct OwnerGroup {
    pub group_key: String,
    pub display_name: String,
    pub display_name_tied: bool,
    pub member_name_count: i64,
    pub total_parcels: Option<i64>,
    pub improvement_value_zero_count: Option<i64>,
    pub municipality_count: i64,
    pub owner_type: Option<String>,
    pub mixed_owner_type: bool,
    pub muni_codes: BTreeSet<String>,
    pub value_as_of_min: Option<String>,
    pub value_as_of_max: Option<String>,
}

struct GroupAcc {
    name_counts: BTreeMap<String, i64>,
    total_parcels: i64,
    zero_count: i64,
    improvement_observed: i64,
    munis: BTreeSet<String>,
    owner_types: BTreeSet<Option<String>>,
    value_as_of_min: Option<String>,
    value_as_of_max: Option<String>,
}

impl GroupAcc {
    fn new() -> Self {
        Self {
            name_counts: BTreeMap::new(),
            total_parcels: 0,
            zero_count: 0,
            improvement_observed: 0,
            munis: BTreeSet::new(),
            owner_types: BTreeSet::new(),
            value_as_of_min: None,
            value_as_of_max: None,
        }
    }
}

pub fn fetch_owner_groups(
    conn: &Connection,
    owner_type: &OwnerTypeFile,
) -> Result<Vec<OwnerGroup>, StoreError> {
    fetch_owner_groups_in_county(conn, None, owner_type)
}

pub fn fetch_owner_groups_in_county(
    conn: &Connection,
    county_fips: Option<&str>,
    owner_type: &OwnerTypeFile,
) -> Result<Vec<OwnerGroup>, StoreError> {
    let sql = match county_fips {
        Some(_) => {
            "SELECT owner_name, muni_code, improvement_value, value_as_of
             FROM current_parcels
             WHERE county_fips = ?1"
        }
        None => {
            "SELECT owner_name, muni_code, improvement_value, value_as_of
             FROM current_parcels"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let mut rows = match county_fips {
        Some(fips) => stmt.query(rusqlite::params![fips])?,
        None => stmt.query([])?,
    };

    let mut by_key: BTreeMap<String, GroupAcc> = BTreeMap::new();
    let mut source_rows = 0i64;
    let mut grouped_rows = 0i64;
    while let Some(row) = rows.next()? {
        source_rows += 1;
        let owner_name: Option<String> = row.get(0)?;
        let muni_code: Option<String> = row.get(1)?;
        let improvement_value: Option<f64> = row.get(2)?;
        let value_as_of: Option<String> = row.get(3)?;
        let Some(raw) = owner_name else {
            continue;
        };
        let key = owner_group_key(&raw);
        if key.is_empty() {
            continue;
        }
        grouped_rows += 1;
        let resolved = owner_type.classify(&raw);
        let acc = by_key.entry(key).or_insert_with(GroupAcc::new);
        *acc.name_counts.entry(raw).or_insert(0) += 1;
        acc.total_parcels += 1;
        match improvement_value {
            Some(0.0) => {
                acc.improvement_observed += 1;
                acc.zero_count += 1;
            }
            Some(_) => {
                acc.improvement_observed += 1;
            }
            None => {}
        }
        if let Some(muni) = muni_code {
            acc.munis.insert(muni);
        }
        acc.owner_types.insert(Some(resolved));
        if let Some(date) = value_as_of {
            acc.value_as_of_min = Some(match acc.value_as_of_min.take() {
                Some(existing) => existing.min(date.clone()),
                None => date.clone(),
            });
            acc.value_as_of_max = Some(match acc.value_as_of_max.take() {
                Some(existing) => existing.max(date),
                None => date,
            });
        }
    }
    debug_assert!(grouped_rows <= source_rows);

    let mut groups = Vec::with_capacity(by_key.len());
    for (group_key, acc) in by_key {
        groups.push(finish_group(group_key, acc));
    }
    Ok(groups)
}

fn finish_group(group_key: String, acc: GroupAcc) -> OwnerGroup {
    let member_name_count = acc.name_counts.len() as i64;
    let max_freq = acc.name_counts.values().copied().max().unwrap_or(0);
    let tied = acc
        .name_counts
        .values()
        .filter(|count| **count == max_freq)
        .count()
        > 1;
    let display_name = acc
        .name_counts
        .iter()
        .filter(|(_, count)| **count == max_freq)
        .map(|(name, _)| name.clone())
        .min()
        .unwrap_or_default();

    let mixed_owner_type = acc.owner_types.len() > 1;
    let owner_type = if mixed_owner_type {
        None
    } else {
        acc.owner_types.into_iter().next().flatten()
    };

    OwnerGroup {
        group_key,
        display_name,
        display_name_tied: tied,
        member_name_count,
        total_parcels: Some(acc.total_parcels),
        improvement_value_zero_count: if acc.improvement_observed == 0 {
            None
        } else {
            Some(acc.zero_count)
        },
        municipality_count: acc.munis.len() as i64,
        owner_type,
        mixed_owner_type,
        muni_codes: acc.munis,
        value_as_of_min: acc.value_as_of_min,
        value_as_of_max: acc.value_as_of_max,
    }
}

pub fn format_value_as_of_range(min: Option<&str>, max: Option<&str>) -> Option<String> {
    match (min, max) {
        (None, None) => None,
        (Some(a), None) | (None, Some(a)) => Some(a.to_string()),
        (Some(a), Some(b)) if a == b => Some(a.to_string()),
        (Some(a), Some(b)) => Some(format!("{a} to {b}")),
    }
}

// Display aid only. Must not be used to merge groups. Sharing a first
// token is not evidence of a single legal entity.
pub fn related_group_keys_by_first_token(groups: &[OwnerGroup], group_key: &str) -> Vec<String> {
    let Some(token) = first_token(group_key) else {
        return Vec::new();
    };
    groups
        .iter()
        .filter(|group| {
            group.group_key != group_key && first_token(&group.group_key) == Some(token)
        })
        .map(|group| group.group_key.clone())
        .collect()
}

fn first_token(group_key: &str) -> Option<&str> {
    group_key.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use radar_config::OwnerTypeFile;
    use rusqlite::Connection;

    use super::{
        fetch_owner_groups, fetch_owner_groups_in_county, format_value_as_of_range,
        related_group_keys_by_first_token,
    };
    use crate::migrate::migrate;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (source_id, status, finished_at)
             VALUES ('src-test', 'success', '2026-08-10')",
            [],
        )
        .expect("load_run");
        conn
    }

    fn insert_entity(
        conn: &Connection,
        taxkey: &str,
        owner_name: &str,
        owner_type: Option<&str>,
        muni_code: Option<&str>,
        improvement_value: Option<f64>,
    ) {
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value,
                 county_fips, value_as_of
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, '55133', '2026-08-10')",
            rusqlite::params![taxkey, owner_name, owner_type, muni_code, improvement_value],
        )
        .expect("insert entity");
        crate::current::mirror_latest_entity(conn).expect("mirror current_parcels");
    }

    #[test]
    fn display_name_picks_most_frequent_raw_name() {
        let conn = setup();
        insert_entity(&conn, "a1", "BIELINSKI HOMES INC", Some("entity"), Some("WAKC"), Some(1.0));
        insert_entity(&conn, "a2", "BIELINSKI HOMES INC", Some("entity"), Some("WAKC"), Some(1.0));
        insert_entity(&conn, "a3", "BIELINSKI HOMES INC", Some("entity"), Some("WAKC"), Some(0.0));
        insert_entity(&conn, "a4", "BIELINSKI HOMES, INC.", Some("entity"), Some("NBC"), Some(1.0));
        let types = OwnerTypeFile::from_dr_data_file();
        let groups = fetch_owner_groups(&conn, &types).expect("groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].display_name, "BIELINSKI HOMES INC");
        assert!(!groups[0].display_name_tied);
        assert_eq!(groups[0].member_name_count, 2);
        assert_eq!(groups[0].total_parcels, Some(4));
        assert_eq!(groups[0].improvement_value_zero_count, Some(1));
        assert_eq!(groups[0].municipality_count, 2);
    }

    #[test]
    fn display_name_tie_is_recorded() {
        let conn = setup();
        insert_entity(&conn, "a1", "FOO INC", Some("entity"), Some("WAKC"), Some(1.0));
        insert_entity(&conn, "a2", "FOO INC", Some("entity"), Some("WAKC"), Some(1.0));
        insert_entity(&conn, "a3", "FOO, INC.", Some("entity"), Some("WAKC"), Some(1.0));
        insert_entity(&conn, "a4", "FOO, INC.", Some("entity"), Some("WAKC"), Some(1.0));
        let types = OwnerTypeFile::from_dr_data_file();
        let groups = fetch_owner_groups(&conn, &types).expect("groups");
        assert_eq!(groups.len(), 1);
        assert!(groups[0].display_name_tied);
        assert_eq!(groups[0].display_name, "FOO INC");
        assert_eq!(groups[0].member_name_count, 2);
    }

    #[test]
    fn mixed_owner_type_is_null_and_flagged() {
        let conn = setup();
        insert_entity(&conn, "a1", "ACME LLC", Some("entity"), Some("WAKC"), Some(1.0));
        insert_entity(&conn, "a2", "ACME FAMILY TRUST", Some("trust"), Some("WAKC"), Some(1.0));
        let types = OwnerTypeFile::from_dr_data_file();
        let groups = fetch_owner_groups(&conn, &types).expect("groups");
        assert_eq!(groups.len(), 2);
        let llc = groups.iter().find(|g| g.group_key == "ACME LLC").expect("llc");
        let trust = groups.iter().find(|g| g.group_key.contains("TRUST")).expect("trust");
        assert_eq!(llc.owner_type.as_deref(), Some("entity"));
        assert_eq!(trust.owner_type.as_deref(), Some("trust"));
        assert!(!llc.mixed_owner_type);
        assert!(!trust.mixed_owner_type);
    }

    #[test]
    fn stored_owner_type_is_ignored_on_read() {
        let conn = setup();
        insert_entity(&conn, "a1", "WAUKESHA COUNTY", Some("individual"), Some("WAKC"), Some(0.0));
        let types = OwnerTypeFile::from_dr_data_file();
        let groups = fetch_owner_groups(&conn, &types).expect("groups");
        assert_eq!(groups[0].owner_type.as_deref(), Some("government"));
    }

    #[test]
    fn related_first_token_is_not_a_merge() {
        let conn = setup();
        insert_entity(
            &conn,
            "a1",
            "HOME PATH FINANCIAL LIMITED PARTNERSHIP",
            Some("entity"),
            Some("WAKC"),
            Some(1.0),
        );
        insert_entity(
            &conn,
            "a2",
            "HOME PATH FINANCIAL LP",
            Some("entity"),
            Some("WAKC"),
            Some(1.0),
        );
        let types = OwnerTypeFile::from_dr_data_file();
        let groups = fetch_owner_groups(&conn, &types).expect("groups");
        assert_eq!(groups.len(), 2);
        let related = related_group_keys_by_first_token(&groups, &groups[0].group_key);
        assert_eq!(related.len(), 1);
        assert_ne!(groups[0].group_key, groups[1].group_key);
    }

    #[test]
    fn county_scope_excludes_other_county_parcels() {
        let conn = setup();
        insert_entity(&conn, "a1", "ACME LLC", Some("entity"), Some("WAKC"), Some(1.0));
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value,
                 county_fips, value_as_of
             ) VALUES (1, 'b1', 'OTHER LLC', 'entity', '53000', 1, '55079', '2026-08-10')",
            [],
        )
        .expect("other county");
        crate::current::mirror_latest_entity(&conn).expect("mirror");
        let types = OwnerTypeFile::from_dr_data_file();
        let scoped = fetch_owner_groups_in_county(&conn, Some("55133"), &types).expect("scoped");
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].group_key, "ACME LLC");
        let all = fetch_owner_groups(&conn, &types).expect("all");
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn value_as_of_range_is_single_date_when_parcels_agree() {
        let conn = setup();
        insert_entity(&conn, "a1", "ACME LLC", Some("entity"), Some("WAKC"), Some(1.0));
        insert_entity(&conn, "a2", "ACME LLC", Some("entity"), Some("WAKC"), Some(0.0));
        let types = OwnerTypeFile::from_dr_data_file();
        let groups = fetch_owner_groups(&conn, &types).expect("groups");
        assert_eq!(groups[0].value_as_of_min.as_deref(), Some("2026-08-10"));
        assert_eq!(groups[0].value_as_of_max.as_deref(), Some("2026-08-10"));
        assert_eq!(
            format_value_as_of_range(
                groups[0].value_as_of_min.as_deref(),
                groups[0].value_as_of_max.as_deref()
            )
            .as_deref(),
            Some("2026-08-10")
        );
    }

    #[test]
    fn null_improvement_is_unknown_not_zero() {
        let conn = setup();
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value,
                 county_fips, value_as_of
             ) VALUES (1, 'n1', 'NULL LLC', 'entity', 'WAKC', NULL, '55133', '2026-08-10')",
            [],
        )
        .expect("null improvement");
        crate::current::mirror_latest_entity(&conn).expect("mirror");
        let types = OwnerTypeFile::from_dr_data_file();
        let groups = fetch_owner_groups(&conn, &types).expect("groups");
        assert_eq!(groups[0].total_parcels, Some(1));
        assert_eq!(groups[0].improvement_value_zero_count, None);
    }

    #[test]
    fn value_as_of_range_prints_when_parcels_disagree() {
        assert_eq!(
            format_value_as_of_range(Some("2025-01-01"), Some("2026-08-10")).as_deref(),
            Some("2025-01-01 to 2026-08-10")
        );
        assert_eq!(format_value_as_of_range(None, None), None);
    }
}
