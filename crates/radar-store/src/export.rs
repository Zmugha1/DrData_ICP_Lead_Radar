use std::fs;
use std::path::Path;

use radar_config::{Favourability, RadarConfig, Zone};
use rusqlite::Connection;

use crate::rank::{rank_groups, RankError, RankedGroup, TRIGGER_STATUS_UNWIRED};

pub fn export_ranked_csv(
    config: &RadarConfig,
    conn: &Connection,
    path: &Path,
) -> Result<Vec<RankedGroup>, RankError> {
    let ranked = rank_groups(config, conn)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let mut csv = String::new();
    csv.push_str(
        "rank,in_working_set,display_name,group_key,member_name_count,total_parcels,improvement_value_zero_count,band_label,band_favourability,municipality_count,trigger_status\n",
    );
    for row in &ranked {
        csv.push_str(&csv_row(&[
            row.rank.to_string(),
            if row.in_working_set {
                "true".to_string()
            } else {
                "false".to_string()
            },
            row.display_name.clone(),
            row.group_key.clone(),
            row.member_name_count.to_string(),
            option_i64(row.total_parcels),
            option_i64(row.improvement_value_zero_count),
            row.band_label.clone().unwrap_or_default(),
            favourability_cell(row.band_favourability.as_ref()),
            row.municipality_count.to_string(),
            TRIGGER_STATUS_UNWIRED.to_string(),
        ]));
        csv.push('\n');
    }
    fs::write(path, csv)?;

    let provenance_path = provenance_path(path);
    let provenance = provenance_text(config, conn, ranked.len())?;
    fs::write(&provenance_path, provenance)?;
    Ok(ranked)
}

fn provenance_path(csv_path: &Path) -> std::path::PathBuf {
    let stem = csv_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".to_string());
    match csv_path.parent() {
        Some(parent) => parent.join(format!("{stem}.provenance.txt")),
        None => Path::new(&format!("{stem}.provenance.txt")).to_path_buf(),
    }
}

fn provenance_text(
    config: &RadarConfig,
    conn: &Connection,
    row_count: usize,
) -> Result<String, RankError> {
    let exported_at: String = conn
        .query_row("SELECT datetime('now')", [], |row| row.get(0))
        .map_err(|err| RankError::Store(err.into()))?;

    let mut runs = String::new();
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT e.load_run_id, e.value_as_of
             FROM current_entities e
             ORDER BY e.load_run_id, e.value_as_of",
        )
        .map_err(|err| RankError::Store(err.into()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|err| RankError::Store(err.into()))?;
    let mut match_count = 0i64;
    for row in rows {
        let (id, value_as_of) = row.map_err(|err| RankError::Store(err.into()))?;
        match_count += 1;
        runs.push_str(&format!("load_run_id={id} value_as_of={value_as_of:?}\n"));
    }
    println!("current_entities load_run distinct pairs: {match_count}");

    let caveat = config
        .zones
        .assignments
        .iter()
        .find(|assignment| {
            assignment.capability_key == "prospect_ranking_by_likelihood_to_buy"
                && assignment.zone == Zone::Below
        })
        .and_then(|assignment| assignment.display_caveat.clone())
        .unwrap_or_default();

    Ok(format!(
        "exported_at={exported_at}\nconfig_version={}\nrows_exported={row_count}\n{runs}display_caveat={caveat}\n",
        config.manifest.config_version
    ))
}

fn option_i64(value: Option<i64>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => String::new(),
    }
}

fn favourability_cell(value: Option<&Favourability>) -> String {
    match value {
        Some(Favourability::Favourable) => "Favourable".to_string(),
        Some(Favourability::Neutral) => "Neutral".to_string(),
        Some(Favourability::Unfavourable) => "Unfavourable".to_string(),
        None => String::new(),
    }
}

fn csv_row(fields: &[String]) -> String {
    fields
        .iter()
        .map(|field| escape_csv(field))
        .collect::<Vec<_>>()
        .join(",")
}

fn escape_csv(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::export_ranked_csv;
    use crate::migrate::migrate;
    use crate::rank::tests::{test_bands, test_config};
    use crate::rank::TRIGGER_STATUS_UNWIRED;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at, value_as_of)
             VALUES (1, 'src-test', 'success', '2026-08-10', '2026-08-10')",
            [],
        )
        .expect("run");
        conn.execute(
            "INSERT INTO entities (load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value, value_as_of, county_fips)
             VALUES (1, 't1', 'ACME LLC', 'entity', 'WAKC', 0, '2026-08-10', '55133')",
            [],
        )
        .expect("entity");
        conn.execute(
            "INSERT INTO entities (load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value, value_as_of, county_fips)
             VALUES (1, 't2', 'BETA LLC', 'entity', 'WAKC', 0, '2026-08-10', '55133')",
            [],
        )
        .expect("entity");
        crate::current::rebuild_current_parcels(&conn, None).expect("rebuild");
        conn
    }

    #[test]
    fn csv_rows_match_ranking_plus_header_and_trigger_unknown() {
        let conn = setup();
        let caveat = "TEST CAVEAT FROM CONFIG";
        let config = test_config(1, caveat, test_bands());
        let dir = std::env::temp_dir().join(format!(
            "radar_export_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        let csv_path = dir.join("ranked.csv");
        let ranked = export_ranked_csv(&config, &conn, &csv_path).expect("export");
        let body = fs::read_to_string(&csv_path).expect("read csv");
        let lines: Vec<&str> = body.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(lines.len(), ranked.len() + 1);
        assert!(lines[0].starts_with("rank,in_working_set,"));
        for line in lines.iter().skip(1) {
            assert!(
                line.ends_with(&format!(",{TRIGGER_STATUS_UNWIRED}")),
                "trigger_status missing on {line}"
            );
        }
        let provenance = fs::read_to_string(dir.join("ranked.provenance.txt")).expect("provenance");
        assert!(
            provenance.contains(caveat),
            "provenance missing caveat from config: {provenance}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
