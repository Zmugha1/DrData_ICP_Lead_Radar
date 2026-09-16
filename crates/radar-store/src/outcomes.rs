// Outcomes are a record of an action against a subject. They are never
// a status on the subject. A subject can carry many outcomes over time
// and none is superseded. Insert only; a mistake is another row.
//
// Recording is the whole job until there are months of data. Nothing
// in this crate may read outcomes to rank, score, convert, predict or
// suggest. This table is the record that will eventually inform the
// Below Zone assignment prospect_ranking_by_likelihood_to_buy. Until
// then, capture only.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::series::{days_between, today_ymd};
use crate::StoreError;

pub const SUBJECT_OWNER_GROUP: &str = "owner_group";
pub const SUBJECT_ASSIGNMENT_ASSIGNEE: &str = "assignment_assignee";

pub const OUTCOME_ACTIONS: &[&str] = &[
    "contacted",
    "met",
    "proposed",
    "closed_won",
    "closed_lost",
    "not_a_fit",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordOutcomeRequest {
    pub subject_type: String,
    pub subject_key: String,
    pub county_fips: Option<String>,
    pub action: String,
    pub occurred_on: Option<String>,
    pub reason: Option<String>,
    pub config_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeRecord {
    pub id: i64,
    pub subject_type: String,
    pub subject_key: String,
    pub county_fips: Option<String>,
    pub action: String,
    pub occurred_on: String,
    pub reason: Option<String>,
    pub config_fingerprint: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatestOutcome {
    pub action: String,
    pub occurred_on: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeHistory {
    pub subject_type: String,
    pub subject_key: String,
    pub today: String,
    pub outcomes: Vec<OutcomeRecord>,
}

pub fn assignment_subject_key(reel_no: &str, frame_no: &str, sequence: i64) -> String {
    format!("{reel_no}/{frame_no}/{sequence}")
}

pub fn parse_assignment_subject_key(key: &str) -> Result<(String, String, i64), StoreError> {
    let mut parts = key.rsplitn(3, '/');
    let sequence = parts.next().ok_or_else(|| invalid_key())?;
    let frame_no = parts.next().ok_or_else(|| invalid_key())?;
    let reel_no = parts.next().ok_or_else(|| invalid_key())?;
    if reel_no.is_empty() || frame_no.is_empty() {
        return Err(invalid_key());
    }
    let sequence = sequence
        .parse::<i64>()
        .map_err(|_| invalid_key())?;
    Ok((reel_no.to_string(), frame_no.to_string(), sequence))
}

fn invalid_key() -> StoreError {
    StoreError::Outcome(
        "assignment_assignee subject_key must be reel_no/frame_no/sequence".to_string(),
    )
}

pub fn county_fips_for_assignment_county(
    county: Option<&str>,
    unmatched: bool,
) -> Option<String> {
    if unmatched {
        return None;
    }
    match county {
        Some("waukesha") => Some("55133".to_string()),
        Some("milwaukee") => Some("55079".to_string()),
        _ => None,
    }
}

pub fn insert_outcome(
    conn: &Connection,
    request: &RecordOutcomeRequest,
) -> Result<OutcomeRecord, StoreError> {
    let subject_type = normalised_subject_type(&request.subject_type)?;
    let subject_key = request.subject_key.trim();
    if subject_key.is_empty() {
        return Err(StoreError::Outcome("subject_key is empty".to_string()));
    }
    if subject_type == SUBJECT_ASSIGNMENT_ASSIGNEE {
        parse_assignment_subject_key(subject_key)?;
    }
    let action = normalised_action(&request.action)?;
    let occurred_on = match request.occurred_on.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            let date = value.trim();
            validate_ymd(date)?;
            date.to_string()
        }
        _ => today_ymd(),
    };
    let reason = request.reason.as_deref().and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let fingerprint = request.config_fingerprint.trim();
    if fingerprint.is_empty() {
        return Err(StoreError::Outcome("config_fingerprint is empty".to_string()));
    }
    let county_fips = request.county_fips.as_deref().and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    conn.execute(
        "INSERT INTO outcomes (
             subject_type, subject_key, county_fips, action, occurred_on,
             reason, config_fingerprint, recorded_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
        params![
            subject_type,
            subject_key,
            county_fips,
            action,
            occurred_on,
            reason,
            fingerprint,
        ],
    )?;
    let id = conn.last_insert_rowid();
    read_outcome(conn, id)
}

pub fn outcome_history(
    conn: &Connection,
    subject_type: &str,
    subject_key: &str,
) -> Result<OutcomeHistory, StoreError> {
    let subject_type = normalised_subject_type(subject_type)?;
    let subject_key = subject_key.trim();
    let mut stmt = conn.prepare(
        "SELECT id, subject_type, subject_key, county_fips, action, occurred_on,
                reason, config_fingerprint, recorded_at
         FROM outcomes
         WHERE subject_type = ?1 AND subject_key = ?2
         ORDER BY occurred_on ASC, recorded_at ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![subject_type, subject_key], map_outcome)?;
    let mut outcomes = Vec::new();
    for row in rows {
        outcomes.push(row?);
    }
    Ok(OutcomeHistory {
        subject_type: subject_type.to_string(),
        subject_key: subject_key.to_string(),
        today: today_ymd(),
        outcomes,
    })
}

pub fn latest_outcomes(
    conn: &Connection,
    subject_type: &str,
    keys: &[String],
) -> Result<HashMap<String, LatestOutcome>, StoreError> {
    let mut latest = HashMap::new();
    if keys.is_empty() {
        return Ok(latest);
    }
    let subject_type = normalised_subject_type(subject_type)?;
    let wanted: std::collections::HashSet<&str> = keys.iter().map(String::as_str).collect();
    let mut stmt = conn.prepare(
        "SELECT subject_key, action, occurred_on
         FROM outcomes
         WHERE subject_type = ?1
         ORDER BY occurred_on DESC, recorded_at DESC, id DESC",
    )?;
    let rows = stmt.query_map([subject_type], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (subject_key, action, occurred_on) = row?;
        if wanted.contains(subject_key.as_str()) {
            latest.entry(subject_key).or_insert(LatestOutcome {
                action,
                occurred_on,
            });
        }
    }
    Ok(latest)
}

fn read_outcome(conn: &Connection, id: i64) -> Result<OutcomeRecord, StoreError> {
    conn.query_row(
        "SELECT id, subject_type, subject_key, county_fips, action, occurred_on,
                reason, config_fingerprint, recorded_at
         FROM outcomes WHERE id = ?1",
        [id],
        map_outcome,
    )
    .map_err(StoreError::from)
}

fn map_outcome(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutcomeRecord> {
    Ok(OutcomeRecord {
        id: row.get(0)?,
        subject_type: row.get(1)?,
        subject_key: row.get(2)?,
        county_fips: row.get(3)?,
        action: row.get(4)?,
        occurred_on: row.get(5)?,
        reason: row.get(6)?,
        config_fingerprint: row.get(7)?,
        recorded_at: row.get(8)?,
    })
}

fn normalised_subject_type(value: &str) -> Result<&'static str, StoreError> {
    match value.trim() {
        SUBJECT_OWNER_GROUP => Ok(SUBJECT_OWNER_GROUP),
        SUBJECT_ASSIGNMENT_ASSIGNEE => Ok(SUBJECT_ASSIGNMENT_ASSIGNEE),
        other => Err(StoreError::Outcome(format!(
            "unknown subject_type {other}"
        ))),
    }
}

fn normalised_action(value: &str) -> Result<&'static str, StoreError> {
    let trimmed = value.trim();
    OUTCOME_ACTIONS
        .iter()
        .copied()
        .find(|action| *action == trimmed)
        .ok_or_else(|| StoreError::Outcome(format!("unknown action {trimmed}")))
}

fn validate_ymd(value: &str) -> Result<(), StoreError> {
    if days_between(value, value).is_some() {
        Ok(())
    } else {
        Err(StoreError::Outcome(format!(
            "occurred_on must be YYYY-MM-DD, got {value}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::{list_owner_groups, GroupListQuery};
    use crate::rank::tests::{test_bands, test_config};
    use crate::rank::rank_groups;
    use crate::migrate::migrate;
    use rusqlite::Connection;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    fn request(
        subject_type: &str,
        subject_key: &str,
        action: &str,
        occurred_on: Option<&str>,
        reason: Option<&str>,
    ) -> RecordOutcomeRequest {
        RecordOutcomeRequest {
            subject_type: subject_type.to_string(),
            subject_key: subject_key.to_string(),
            county_fips: Some("55133".to_string()),
            action: action.to_string(),
            occurred_on: occurred_on.map(str::to_string),
            reason: reason.map(str::to_string),
            config_fingerprint: "test-fingerprint".to_string(),
        }
    }

    fn add_group(conn: &Connection, name: &str) {
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM load_runs WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("runs");
        if runs == 0 {
            conn.execute(
                "INSERT INTO load_runs (id, source_id, status, finished_at, value_as_of)
                 VALUES (1, 'src-test', 'success', '2026-08-10', '2026-08-10')",
                [],
            )
            .expect("run");
        }
        conn.execute(
            "INSERT INTO entities (
                 load_run_id, taxkey, owner_name, owner_type, muni_code,
                 improvement_value, county_fips, value_as_of
             ) VALUES (1, ?1, ?2, 'entity', 'WAKC', 0, '55133', '2026-08-10')",
            rusqlite::params![format!("{name}-1"), name],
        )
        .expect("entity");
        crate::current::mirror_latest_entity(conn).expect("mirror");
    }

    #[test]
    fn outcome_inserts_and_appears_in_history() {
        let conn = open();
        let recorded = insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "ACME LLC",
                "contacted",
                Some("2026-09-01"),
                Some("called the office"),
            ),
        )
        .expect("insert");
        assert_eq!(recorded.action, "contacted");
        assert_eq!(recorded.occurred_on, "2026-09-01");
        assert_eq!(recorded.reason.as_deref(), Some("called the office"));
        let history = outcome_history(&conn, SUBJECT_OWNER_GROUP, "ACME LLC").expect("history");
        assert_eq!(history.outcomes.len(), 1);
        assert_eq!(history.outcomes[0].id, recorded.id);
        assert_eq!(history.outcomes[0].config_fingerprint, "test-fingerprint");
    }

    #[test]
    fn second_outcome_does_not_replace_the_first() {
        let conn = open();
        let first = insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "ACME LLC",
                "contacted",
                Some("2026-01-15"),
                None,
            ),
        )
        .expect("first");
        let second = insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "ACME LLC",
                "met",
                Some("2026-03-01"),
                Some("site walk"),
            ),
        )
        .expect("second");
        assert_ne!(first.id, second.id);
        let history = outcome_history(&conn, SUBJECT_OWNER_GROUP, "ACME LLC").expect("history");
        assert_eq!(history.outcomes.len(), 2);
        assert_eq!(history.outcomes[0].action, "contacted");
        assert_eq!(history.outcomes[0].occurred_on, "2026-01-15");
        assert_eq!(history.outcomes[1].action, "met");
        assert_eq!(history.outcomes[1].occurred_on, "2026-03-01");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outcomes WHERE subject_key = 'ACME LLC'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 2);
    }

    #[test]
    fn occurred_on_accepts_a_past_date() {
        let conn = open();
        let recorded = insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "LOST CLIENT LLC",
                "closed_lost",
                Some("2024-03-15"),
                Some("existing loss, backfilled"),
            ),
        )
        .expect("insert");
        assert_eq!(recorded.occurred_on, "2024-03-15");
        let today = today_ymd();
        assert!(recorded.occurred_on < today);
    }

    #[test]
    fn subject_with_no_outcomes_shows_nothing_not_a_default() {
        let conn = open();
        add_group(&conn, "ACME LLC");
        add_group(&conn, "BETA LLC");
        insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "ACME LLC",
                "proposed",
                Some("2026-06-01"),
                None,
            ),
        )
        .expect("insert");
        let page = list_owner_groups(
            &test_config(1, "caveat", test_bands()),
            &conn,
            &GroupListQuery::default(),
        )
        .expect("page");
        let acme = page
            .rows
            .iter()
            .find(|row| row.group_key == "ACME LLC")
            .expect("acme");
        let beta = page
            .rows
            .iter()
            .find(|row| row.group_key == "BETA LLC")
            .expect("beta");
        assert_eq!(
            acme.latest_outcome,
            Some(LatestOutcome {
                action: "proposed".to_string(),
                occurred_on: "2026-06-01".to_string(),
            })
        );
        assert_eq!(beta.latest_outcome, None);
        let empty = outcome_history(&conn, SUBJECT_OWNER_GROUP, "BETA LLC").expect("history");
        assert!(empty.outcomes.is_empty());
    }

    #[test]
    fn nothing_reads_outcomes_for_ranking() {
        let src = include_str!("rank.rs");
        assert!(
            !src.contains("FROM outcomes") && !src.contains("JOIN outcomes"),
            "rank.rs must not query outcomes"
        );
        let conn = open();
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at, value_as_of)
             VALUES (1, 'src-test', 'success', '2026-08-10', '2026-08-10')",
            [],
        )
        .expect("run");
        for (name, vacant, extra) in [("ALPHA LLC", 10, 0), ("BETA LLC", 8, 12)] {
            for n in 0..vacant {
                conn.execute(
                    "INSERT INTO entities (load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value, county_fips)
                     VALUES (1, ?1, ?2, 'entity', 'WAKC', 0, '55133')",
                    rusqlite::params![format!("{name}-v{n}"), name],
                )
                .expect("vacant");
                crate::current::mirror_latest_entity(&conn).expect("mirror");
            }
            for n in 0..extra {
                conn.execute(
                    "INSERT INTO entities (load_run_id, taxkey, owner_name, owner_type, muni_code, improvement_value, county_fips)
                     VALUES (1, ?1, ?2, 'entity', 'WAKC', 100, '55133')",
                    rusqlite::params![format!("{name}-b{n}"), name],
                )
                .expect("built");
                crate::current::mirror_latest_entity(&conn).expect("mirror");
            }
        }
        let config = test_config(1, "caveat", test_bands());
        let before = rank_groups(&config, &conn).expect("rank before");
        insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "BETA LLC",
                "closed_won",
                Some("2020-01-01"),
                None,
            ),
        )
        .expect("won on lower group");
        insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "ALPHA LLC",
                "closed_lost",
                Some("2020-01-01"),
                None,
            ),
        )
        .expect("lost on higher group");
        let after = rank_groups(&config, &conn).expect("rank after");
        let before_keys: Vec<&str> = before.iter().map(|row| row.group_key.as_str()).collect();
        let after_keys: Vec<&str> = after.iter().map(|row| row.group_key.as_str()).collect();
        assert_eq!(before_keys, after_keys);
        assert_eq!(before_keys, vec!["ALPHA LLC", "BETA LLC"]);
    }

    #[test]
    fn latest_uses_occurred_on_not_insert_order() {
        let conn = open();
        insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "ACME LLC",
                "closed_won",
                Some("2026-08-01"),
                None,
            ),
        )
        .expect("newer fact first");
        insert_outcome(
            &conn,
            &request(
                SUBJECT_OWNER_GROUP,
                "ACME LLC",
                "contacted",
                Some("2023-01-01"),
                Some("backfill"),
            ),
        )
        .expect("older backfill later");
        let latest = latest_outcomes(&conn, SUBJECT_OWNER_GROUP, &["ACME LLC".to_string()]).expect("latest");
        assert_eq!(latest["ACME LLC"].action, "closed_won");
        assert_eq!(latest["ACME LLC"].occurred_on, "2026-08-01");
    }

    #[test]
    fn outcomes_are_insert_only() {
        let conn = open();
        let recorded = insert_outcome(
            &conn,
            &request(
                SUBJECT_ASSIGNMENT_ASSIGNEE,
                "1/2/1",
                "met",
                Some("2025-05-05"),
                None,
            ),
        )
        .expect("insert");
        let denied = conn.execute(
            "UPDATE outcomes SET action = 'contacted' WHERE id = ?1",
            [recorded.id],
        );
        assert!(denied.is_err(), "expected deny on outcomes update, got {denied:?}");
        let denied = conn.execute("DELETE FROM outcomes WHERE id = ?1", [recorded.id]);
        assert!(denied.is_err(), "expected deny on outcomes delete, got {denied:?}");
    }
}
