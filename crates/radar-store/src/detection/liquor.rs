// The file lists currently active licences and is overwritten
// nightly. It carries no licence number and no new-versus-renewed
// flag. A first-time licence and an annual renewal both appear as a
// row with a fresh effective date.
// A row appearing today and absent yesterday is new to the file. It
// is usually a new licence and may be a record correction.
// The first snapshot establishes a baseline and produces no signal.
//
// Diff keys are (taxkey, lic_type). That pair is not unique in the
// published file. Without a licence number the detector treats the
// set of licensee strings on that pair as the identity of the row.

use rusqlite::Connection;

use crate::liquor::{
    licence_index, liquor_rows, liquor_rows_for_keys, milwaukee_parcels_for_taxkey, snapshot_dates,
    snapshots_every_row_matches, LicenceKey, LiquorRow, DETECTION_PRINT_LIMIT, LIQUOR_CANNOT_TELL,
};
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiquorComparability {
    Comparable,
    SameSnapshotDate { date: String },
    InsufficientSnapshots { count: usize },
}

impl LiquorComparability {
    pub fn label(&self) -> &'static str {
        match self {
            LiquorComparability::Comparable => "comparable",
            LiquorComparability::SameSnapshotDate { .. }
            | LiquorComparability::InsufficientSnapshots { .. } => "not comparable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquorDiff {
    pub comparability: LiquorComparability,
    pub earlier_date: Option<String>,
    pub later_date: Option<String>,
    pub new_count: i64,
    pub ended_count: i64,
    pub changed_count: i64,
    pub placeholders_earlier: i64,
    pub placeholders_later: i64,
    pub new_keys: Vec<LicenceKey>,
    pub ended_keys: Vec<LicenceKey>,
    pub changed_keys: Vec<LicenceKey>,
}

pub fn detect_liquor_pair(
    conn: &Connection,
    earlier_date: &str,
    later_date: &str,
) -> Result<LiquorDiff, StoreError> {
    let (earlier, placeholders_earlier) = licence_index(conn, earlier_date)?;
    let (later, placeholders_later) = licence_index(conn, later_date)?;
    if earlier_date == later_date {
        return Ok(LiquorDiff {
            comparability: LiquorComparability::SameSnapshotDate {
                date: earlier_date.to_string(),
            },
            earlier_date: Some(earlier_date.to_string()),
            later_date: Some(later_date.to_string()),
            new_count: 0,
            ended_count: 0,
            changed_count: 0,
            placeholders_earlier,
            placeholders_later,
            new_keys: Vec::new(),
            ended_keys: Vec::new(),
            changed_keys: Vec::new(),
        });
    }
    let mut new_keys = Vec::new();
    let mut ended_keys = Vec::new();
    let mut changed_keys = Vec::new();
    for key in later.keys() {
        if !earlier.contains_key(key) {
            new_keys.push(key.clone());
        }
    }
    for key in earlier.keys() {
        if !later.contains_key(key) {
            ended_keys.push(key.clone());
        }
    }
    for (key, earlier_names) in &earlier {
        if let Some(later_names) = later.get(key) {
            if earlier_names != later_names {
                changed_keys.push(key.clone());
            }
        }
    }
    Ok(LiquorDiff {
        comparability: LiquorComparability::Comparable,
        earlier_date: Some(earlier_date.to_string()),
        later_date: Some(later_date.to_string()),
        new_count: new_keys.len() as i64,
        ended_count: ended_keys.len() as i64,
        changed_count: changed_keys.len() as i64,
        placeholders_earlier,
        placeholders_later,
        new_keys,
        ended_keys,
        changed_keys,
    })
}

pub fn detect_liquor_latest(conn: &Connection) -> Result<LiquorDiff, StoreError> {
    let dates = snapshot_dates(conn)?;
    match dates.as_slice() {
        [] => Ok(empty_not_comparable(0)),
        [only] => {
            let (_, placeholders) = licence_index(conn, only)?;
            Ok(LiquorDiff {
                comparability: LiquorComparability::InsufficientSnapshots { count: 1 },
                earlier_date: Some(only.clone()),
                later_date: None,
                new_count: 0,
                ended_count: 0,
                changed_count: 0,
                placeholders_earlier: placeholders,
                placeholders_later: 0,
                new_keys: Vec::new(),
                ended_keys: Vec::new(),
                changed_keys: Vec::new(),
            })
        }
        _ => {
            let earlier = &dates[dates.len() - 2];
            let later = &dates[dates.len() - 1];
            detect_liquor_pair(conn, earlier, later)
        }
    }
}

fn empty_not_comparable(count: usize) -> LiquorDiff {
    LiquorDiff {
        comparability: LiquorComparability::InsufficientSnapshots { count },
        earlier_date: None,
        later_date: None,
        new_count: 0,
        ended_count: 0,
        changed_count: 0,
        placeholders_earlier: 0,
        placeholders_later: 0,
        new_keys: Vec::new(),
        ended_keys: Vec::new(),
        changed_keys: Vec::new(),
    }
}

pub fn print_liquor_diff(diff: &LiquorDiff) {
    println!("{LIQUOR_CANNOT_TELL}");
    match &diff.comparability {
        LiquorComparability::Comparable => {
            println!(
                "comparability={} comparability_guard=passed earlier={} later={}",
                diff.comparability.label(),
                diff.earlier_date.as_deref().unwrap_or(""),
                diff.later_date.as_deref().unwrap_or("")
            );
        }
        LiquorComparability::SameSnapshotDate { date } => {
            println!(
                "comparability={} reason=same snapshot_date={date} (different fact from zero changes)",
                diff.comparability.label()
            );
        }
        LiquorComparability::InsufficientSnapshots { count } => {
            println!(
                "comparability={} reason=one snapshot alone is a baseline, not zero changes; snapshot_count={count}",
                diff.comparability.label()
            );
        }
    }
    println!(
        "placeholder taxkey 0000000000 excluded earlier={} later={}",
        diff.placeholders_earlier, diff.placeholders_later
    );
    println!(
        "new_liquor_licence={} liquor_licence_ended={} licensee_changed={}",
        diff.new_count, diff.ended_count, diff.changed_count
    );
}

fn opt_text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("")
}

fn print_licence_line(prefix: &str, index: usize, total: usize, row: &LiquorRow) {
    println!(
        "{prefix} {index}/{total} trade_name={} corp_name={} licensee={} address={} licence_type={} licence_type_full={} eff_date={} taxkey={}",
        opt_text(&row.trade_name),
        opt_text(&row.corp_name),
        opt_text(&row.licensee),
        opt_text(&row.address),
        opt_text(&row.lic_type),
        opt_text(&row.licence_type_full),
        opt_text(&row.eff_date),
        opt_text(&row.taxkey)
    );
}

pub fn print_liquor_detections(conn: &Connection, diff: &LiquorDiff) -> Result<(), StoreError> {
    let Some(earlier_date) = diff.earlier_date.as_deref() else {
        return Ok(());
    };
    let Some(later_date) = diff.later_date.as_deref() else {
        return Ok(());
    };

    let new_rows = liquor_rows_for_keys(conn, later_date, &diff.new_keys)?;
    let ended_rows = liquor_rows_for_keys(conn, earlier_date, &diff.ended_keys)?;
    let changed_later = liquor_rows_for_keys(conn, later_date, &diff.changed_keys)?;
    let earlier_rows = liquor_rows(conn, earlier_date)?;
    let later_all = liquor_rows(conn, later_date)?;
    if snapshots_every_row_matches(&earlier_rows, &later_all) {
        println!(
            "file_unchanged=true every row matches earlier={} later={}",
            earlier_rows.len(),
            later_all.len()
        );
    } else {
        println!(
            "file_unchanged=false earlier_rows={} later_rows={}",
            earlier_rows.len(),
            later_all.len()
        );
    }

    println!("new_liquor_licence count={}", diff.new_count);
    let new_total = new_rows.len();
    let new_show = new_total.min(DETECTION_PRINT_LIMIT);
    for (i, row) in new_rows.iter().take(new_show).enumerate() {
        print_licence_line("new_liquor_licence", i + 1, new_total, row);
        if let Some(taxkey) = row.taxkey.as_deref() {
            for parcel in milwaukee_parcels_for_taxkey(conn, taxkey)? {
                println!(
                    "  parcel owner_name={} assessed_value={}",
                    parcel.owner_name.as_deref().unwrap_or(""),
                    parcel
                        .assessed_value
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                );
            }
        }
    }
    if new_total > DETECTION_PRINT_LIMIT {
        println!("new_liquor_licence printed={new_show} total={new_total}");
    }

    println!("liquor_licence_ended count={}", diff.ended_count);
    let ended_total = ended_rows.len();
    let ended_show = ended_total.min(DETECTION_PRINT_LIMIT);
    for (i, row) in ended_rows.iter().take(ended_show).enumerate() {
        print_licence_line("liquor_licence_ended", i + 1, ended_total, row);
    }
    if ended_total > DETECTION_PRINT_LIMIT {
        println!("liquor_licence_ended printed={ended_show} total={ended_total}");
    }

    println!("licensee_changed count={}", diff.changed_count);
    let changed_total = changed_later.len();
    let changed_show = changed_total.min(DETECTION_PRINT_LIMIT);
    for (i, row) in changed_later.iter().take(changed_show).enumerate() {
        let taxkey = row.taxkey.clone().unwrap_or_default();
        let lic_type = row.lic_type.clone().unwrap_or_default();
        let earlier_names: Vec<String> = earlier_rows
            .iter()
            .filter(|earlier| {
                earlier.taxkey.as_deref() == Some(taxkey.as_str())
                    && earlier.lic_type.as_deref() == Some(lic_type.as_str())
            })
            .filter_map(|earlier| earlier.licensee.clone())
            .collect();
        println!(
            "licensee_changed {}/{} trade_name={} corp_name={} licensee_earlier={} licensee_later={} address={} licence_type={} licence_type_full={} eff_date={} taxkey={}",
            i + 1,
            changed_total,
            opt_text(&row.trade_name),
            opt_text(&row.corp_name),
            earlier_names.join(" | "),
            opt_text(&row.licensee),
            opt_text(&row.address),
            opt_text(&row.lic_type),
            opt_text(&row.licence_type_full),
            opt_text(&row.eff_date),
            opt_text(&row.taxkey)
        );
    }
    if changed_total > DETECTION_PRINT_LIMIT {
        println!("licensee_changed printed={changed_show} total={changed_total}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{detect_liquor_latest, detect_liquor_pair, LiquorComparability};
    use crate::liquor::{load_liquor_snapshot, parse_liquor_csv, PLACEHOLDER_TAXKEY};
    use crate::migrate::migrate;

    const HEADER: &str = "\"EXP_DATE\",\"EFF_DATE\",\"CORP_NAME\",\"TRADE_NAME\",\"LICENSEE\",\"TAXKEY_NUMBER\",\"HOUSE_NR\",\"SDIR\",\"STREET\",\"STTYPE\",\"ALDERMANIC_DISTRICT\",\"POLICE_DISTRICT\",\"LIC_TYPE\",\"License Type Full Name\",\"TOT_CAP\",\"ROOM_CAP\",\"TAXKEY\"\n";

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    fn row(taxkey: &str, lic: &str, licensee: &str) -> String {
        format!(
            "\"2026-10-22\",\"2025-10-15\",\"CORP\",\"TRADE\",\"{licensee}\",\"{taxkey}\",\"1\",\"N\",\"MAIN\",\"ST\",\"1\",\"1\",\"{lic}\",\"Class A\",\"\",\"\",\"{taxkey}\""
        )
    }

    fn csv(rows: &[&str]) -> String {
        format!("{HEADER}{}", rows.join("\n"))
    }

    fn load(conn: &mut Connection, date: &str, csv_text: &str) {
        let rows = parse_liquor_csv(csv_text).expect("parse");
        load_liquor_snapshot(conn, &rows, date, "fixture.csv", &format!("{date}T00:00:00Z"))
            .expect("load");
    }

    #[test]
    fn new_licence_detects_key_present_later_only() {
        let mut conn = open();
        load(
            &mut conn,
            "2026-09-07",
            &csv(&[&row("1111111111", "ALQML", "ALICE, Agt")]),
        );
        load(
            &mut conn,
            "2026-09-08",
            &csv(&[
                &row("1111111111", "ALQML", "ALICE, Agt"),
                &row("2222222222", "BCTV", "BOB, Agt"),
            ]),
        );
        let diff = detect_liquor_pair(&conn, "2026-09-07", "2026-09-08").expect("diff");
        assert_eq!(diff.comparability, LiquorComparability::Comparable);
        assert_eq!(diff.new_count, 1);
        assert_eq!(diff.ended_count, 0);
        assert_eq!(diff.changed_count, 0);
        assert_eq!(
            diff.new_keys,
            vec![("2222222222".to_string(), "BCTV".to_string())]
        );
    }

    #[test]
    fn ended_licence_detects_key_present_earlier_only() {
        let mut conn = open();
        load(
            &mut conn,
            "2026-09-07",
            &csv(&[
                &row("1111111111", "ALQML", "ALICE, Agt"),
                &row("2222222222", "BCTV", "BOB, Agt"),
            ]),
        );
        load(
            &mut conn,
            "2026-09-08",
            &csv(&[&row("1111111111", "ALQML", "ALICE, Agt")]),
        );
        let diff = detect_liquor_pair(&conn, "2026-09-07", "2026-09-08").expect("diff");
        assert_eq!(diff.comparability, LiquorComparability::Comparable);
        assert_eq!(diff.new_count, 0);
        assert_eq!(diff.ended_count, 1);
        assert_eq!(diff.changed_count, 0);
        assert_eq!(
            diff.ended_keys,
            vec![("2222222222".to_string(), "BCTV".to_string())]
        );
    }

    #[test]
    fn licensee_changed_detects_same_key_different_name() {
        let mut conn = open();
        load(
            &mut conn,
            "2026-09-07",
            &csv(&[&row("1111111111", "ALQML", "ALICE, Agt")]),
        );
        load(
            &mut conn,
            "2026-09-08",
            &csv(&[&row("1111111111", "ALQML", "CAROL, Agt")]),
        );
        let diff = detect_liquor_pair(&conn, "2026-09-07", "2026-09-08").expect("diff");
        assert_eq!(diff.comparability, LiquorComparability::Comparable);
        assert_eq!(diff.new_count, 0);
        assert_eq!(diff.ended_count, 0);
        assert_eq!(diff.changed_count, 1);
        assert_eq!(
            diff.changed_keys,
            vec![("1111111111".to_string(), "ALQML".to_string())]
        );
    }

    #[test]
    fn placeholder_taxkeys_are_excluded_and_counted() {
        let mut conn = open();
        load(
            &mut conn,
            "2026-09-07",
            &csv(&[
                &row("1111111111", "ALQML", "ALICE, Agt"),
                &row(PLACEHOLDER_TAXKEY, "ALQML", "PLACE ONE, Agt"),
                &row(PLACEHOLDER_TAXKEY, "BCTV", "PLACE TWO, Agt"),
            ]),
        );
        load(
            &mut conn,
            "2026-09-08",
            &csv(&[
                &row("1111111111", "ALQML", "ALICE, Agt"),
                &row("2222222222", "BCTV", "BOB, Agt"),
                &row(PLACEHOLDER_TAXKEY, "CWTN", "PLACE THREE, Agt"),
            ]),
        );
        let diff = detect_liquor_pair(&conn, "2026-09-07", "2026-09-08").expect("diff");
        assert_eq!(diff.comparability, LiquorComparability::Comparable);
        assert_eq!(diff.placeholders_earlier, 2);
        assert_eq!(diff.placeholders_later, 1);
        assert_eq!(diff.new_count, 1);
        assert_eq!(diff.ended_count, 0);
        assert_eq!(diff.changed_count, 0);
        assert!(!diff
            .new_keys
            .iter()
            .any(|(taxkey, _)| taxkey == PLACEHOLDER_TAXKEY));
        assert!(!diff
            .ended_keys
            .iter()
            .any(|(taxkey, _)| taxkey == PLACEHOLDER_TAXKEY));
    }

    #[test]
    fn same_snapshot_date_is_not_comparable() {
        let mut conn = open();
        load(
            &mut conn,
            "2026-09-08",
            &csv(&[&row("1111111111", "ALQML", "ALICE, Agt")]),
        );
        let diff = detect_liquor_pair(&conn, "2026-09-08", "2026-09-08").expect("diff");
        assert_eq!(
            diff.comparability,
            LiquorComparability::SameSnapshotDate {
                date: "2026-09-08".to_string()
            }
        );
        assert_eq!(diff.comparability.label(), "not comparable");
        assert_ne!(diff.comparability, LiquorComparability::Comparable);
    }

    #[test]
    fn one_snapshot_alone_is_not_comparable() {
        let mut conn = open();
        load(
            &mut conn,
            "2026-09-08",
            &csv(&[&row("1111111111", "ALQML", "ALICE, Agt")]),
        );
        let diff = detect_liquor_latest(&conn).expect("diff");
        assert_eq!(
            diff.comparability,
            LiquorComparability::InsufficientSnapshots { count: 1 }
        );
        assert_eq!(diff.comparability.label(), "not comparable");
        assert_ne!(diff.comparability, LiquorComparability::Comparable);
    }

    #[test]
    fn identical_files_on_two_dates_are_comparable_with_zero_changes() {
        let mut conn = open();
        let csv_text = csv(&[&row("1111111111", "ALQML", "ALICE, Agt")]);
        load(&mut conn, "2026-09-07", &csv_text);
        load(&mut conn, "2026-09-08", &csv_text);
        let diff = detect_liquor_pair(&conn, "2026-09-07", "2026-09-08").expect("diff");
        assert_eq!(diff.comparability, LiquorComparability::Comparable);
        assert_eq!(diff.new_count, 0);
        assert_eq!(diff.ended_count, 0);
        assert_eq!(diff.changed_count, 0);
    }
}
