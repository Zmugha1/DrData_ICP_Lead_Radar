use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossaryTerm {
    pub term_key: String,
    pub label: String,
    pub body: String,
    pub category: String,
}

/// Result of a glossary read. A missing key is a visible miss, not a blank string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum GlossaryRead {
    All { terms: Vec<GlossaryTerm> },
    Term { term: GlossaryTerm },
    Missing { term_key: String },
}

pub fn read_glossary(
    conn: &Connection,
    term_key: Option<&str>,
) -> Result<GlossaryRead, StoreError> {
    match term_key {
        None => Ok(GlossaryRead::All {
            terms: list_glossary(conn)?,
        }),
        Some(key) => match get_term(conn, key)? {
            Some(term) => Ok(GlossaryRead::Term { term }),
            None => Ok(GlossaryRead::Missing {
                term_key: key.to_string(),
            }),
        },
    }
}

fn list_glossary(conn: &Connection) -> Result<Vec<GlossaryTerm>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT term_key, label, body, category
         FROM term_glossary
         ORDER BY category, term_key",
    )?;
    let rows = stmt.query_map([], row_to_term)?;
    let mut terms = Vec::new();
    for row in rows {
        terms.push(row?);
    }
    Ok(terms)
}

fn get_term(conn: &Connection, term_key: &str) -> Result<Option<GlossaryTerm>, StoreError> {
    let term = conn
        .query_row(
            "SELECT term_key, label, body, category
             FROM term_glossary
             WHERE term_key = ?1",
            [term_key],
            row_to_term,
        )
        .optional()?;
    Ok(term)
}

fn row_to_term(row: &rusqlite::Row<'_>) -> rusqlite::Result<GlossaryTerm> {
    Ok(GlossaryTerm {
        term_key: row.get(0)?,
        label: row.get(1)?,
        body: row.get(2)?,
        category: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{read_glossary, GlossaryRead};
    use crate::migrate::migrate;

    fn open_migrated() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        migrate(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn glossary_seeds_every_required_key() {
        let conn = open_migrated();
        let GlossaryRead::All { terms } = read_glossary(&conn, None).unwrap() else {
            panic!("expected All");
        };
        assert_eq!(terms.len(), 38);
        let keys: Vec<&str> = terms.iter().map(|t| t.term_key.as_str()).collect();
        for key in [
            "inside_zone",
            "below_zone",
            "above_zone",
            "oscillating_zone",
            "taxkey",
            "improvement_value",
            "assessed_value",
            "land_value",
            "prop_class",
            "muni_code",
            "owner_individual",
            "owner_trust",
            "owner_entity",
            "owner_government",
            "placeholder",
            "nonprofit",
            "tribal",
            "band_no_vacant_holdings",
            "band_small_holding",
            "band_parcels_no_improvement_value",
            "load_run",
            "value_as_of",
            "value_source",
            "publication_lag",
            "config_fingerprint",
            "coverage_full",
            "coverage_partial",
            "coverage_none",
            "uncovered_municipality",
            "signal",
            "direction",
            "direction_favourable",
            "direction_unfavourable",
            "direction_ask",
            "detection_run_comparison",
            "detection_nightly_diff",
            "source_status",
            "detection_built",
        ] {
            assert!(keys.contains(&key), "missing {key}");
        }
        for term in &terms {
            assert!(!term.label.is_empty());
            assert!(!term.body.is_empty());
            assert!(!term.body.contains('\u{2014}'));
            assert!(!term.body.contains('\u{2013}'));
        }
    }

    #[test]
    fn glossary_by_key_returns_the_row() {
        let conn = open_migrated();
        match read_glossary(&conn, Some("value_as_of")).unwrap() {
            GlossaryRead::Term { term } => {
                assert_eq!(term.term_key, "value_as_of");
                assert_eq!(term.category, "provenance");
                assert!(!term.body.is_empty());
            }
            other => panic!("expected Term, got {other:?}"),
        }
    }

    #[test]
    fn missing_key_is_not_an_empty_string() {
        let conn = open_migrated();
        match read_glossary(&conn, Some("not_a_term")).unwrap() {
            GlossaryRead::Missing { term_key } => assert_eq!(term_key, "not_a_term"),
            GlossaryRead::Term { term } => panic!("must not invent a term: {}", term.body),
            GlossaryRead::All { .. } => panic!("must not return All for a missing key"),
        }
    }
}
