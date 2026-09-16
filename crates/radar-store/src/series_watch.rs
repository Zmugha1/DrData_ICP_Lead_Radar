use radar_config::{RadarConfig, SeriesEntry};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::series::{
    days_between, declared_cadence, n_most_recent, observation_dates, observed_cadence_kind,
    today_ymd,
};
use crate::StoreError;

const COUNTY_SERIES_ID: &str = "BPPRIV055133";
const RECENT_LIMIT: i64 = 24;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketWatchObs {
    pub observation_date: String,
    pub value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketWatchCard {
    pub series_id: String,
    pub label: String,
    pub geography: String,
    pub observed_frequency: String,
    pub latest_value: Option<f64>,
    pub latest_observation_date: Option<String>,
    pub days_old: Option<i64>,
    pub cadence_days: i64,
    pub stale: bool,
    pub stale_text: Option<String>,
    pub history_text: Option<String>,
    pub recent: Vec<MarketWatchObs>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketWatchPage {
    pub vertical_display_name: String,
    pub county_name: String,
    pub county_fips: String,
    pub config_version: String,
    pub cards: Vec<MarketWatchCard>,
    pub clock_note: String,
    pub grain_note: String,
}

pub fn list_market_watch(
    config: &RadarConfig,
    conn: &Connection,
) -> Result<MarketWatchPage, StoreError> {
    let today = today_ymd();
    let mut cards = Vec::new();
    for entry in config.series.series.iter().filter(|entry| entry.included) {
        cards.push(card_for(conn, entry, &config.manifest.county_name, &today)?);
    }
    cards.sort_by(|a, b| {
        geography_rank(&a.geography)
            .cmp(&geography_rank(&b.geography))
            .then(a.series_id.cmp(&b.series_id))
    });
    Ok(MarketWatchPage {
        vertical_display_name: config.manifest.vertical_display_name.clone(),
        county_name: config.manifest.county_name.clone(),
        county_fips: config.manifest.county_fips.clone(),
        config_version: config.manifest.config_version.clone(),
        clock_note: clock_note(&cards),
        grain_note: "These series are county or national aggregates. They do not join to parcels and are not lead signals.".to_string(),
        cards,
    })
}

fn card_for(
    conn: &Connection,
    entry: &SeriesEntry,
    county_name: &str,
    today: &str,
) -> Result<MarketWatchCard, StoreError> {
    let catalog_frequency: Option<String> = conn
        .query_row(
            "SELECT frequency FROM series_catalog WHERE series_id = ?1",
            [&entry.series_id],
            |row| row.get(0),
        )
        .optional()?;
    let dates = observation_dates(conn, &entry.series_id)?;
    let observed_frequency = if dates.len() >= 2 {
        observed_cadence_kind(&dates).to_string()
    } else {
        declared_cadence(catalog_frequency.as_deref().unwrap_or("")).to_string()
    };
    let cadence_days = cadence_days_for(&observed_frequency);
    let recent_pairs = n_most_recent(conn, &entry.series_id, RECENT_LIMIT)?;
    let recent: Vec<MarketWatchObs> = recent_pairs
        .into_iter()
        .map(|(observation_date, value)| MarketWatchObs {
            observation_date,
            value,
        })
        .collect();
    let latest = recent.first();
    let latest_observation_date = latest.map(|row| row.observation_date.clone());
    let latest_value = latest.and_then(|row| row.value);
    let days_old = latest_observation_date
        .as_deref()
        .and_then(|date| days_between(date, today));
    let stale = match days_old {
        Some(days) => days > cadence_days * 2,
        None => false,
    };
    let stale_text = if stale {
        days_old.map(|days| {
            format!(
                "This observation is {days} days old, which is more than twice the {cadence_days}-day {observed_frequency} cadence."
            )
        })
    } else {
        None
    };
    let history_text = county_history_text(entry, county_name, &latest_observation_date, days_old);
    Ok(MarketWatchCard {
        series_id: entry.series_id.clone(),
        label: entry.label.clone(),
        geography: entry.geography.clone(),
        observed_frequency,
        latest_value,
        latest_observation_date,
        days_old,
        cadence_days,
        stale,
        stale_text,
        history_text,
        recent,
    })
}

fn county_history_text(
    entry: &SeriesEntry,
    county_name: &str,
    latest_date: &Option<String>,
    days_old: Option<i64>,
) -> Option<String> {
    if entry.series_id != COUNTY_SERIES_ID {
        return None;
    }
    let date = latest_date.as_deref()?;
    let days = days_old?;
    let year = if date.len() >= 4 { &date[0..4] } else { date };
    let age = if days > 600 {
        format!("over 600 days old ({days} days)")
    } else {
        format!("{days} days old")
    };
    Some(format!(
        "The newest {county_name} figure is from {year} and is {age}. That is history, not a current reading, and a builder must not read it as this year."
    ))
}

fn cadence_days_for(kind: &str) -> i64 {
    match kind {
        "weekly" => 7,
        "biweekly" => 14,
        "monthly" => 30,
        "quarterly" => 90,
        "annual" => 365,
        _ => 30,
    }
}

fn geography_rank(geography: &str) -> i32 {
    match geography {
        "county" => 0,
        "state" => 1,
        "national" => 2,
        _ => 3,
    }
}

fn clock_note(cards: &[MarketWatchCard]) -> String {
    let weekly: Vec<&MarketWatchCard> = cards
        .iter()
        .filter(|card| card.observed_frequency == "weekly")
        .collect();
    let monthly: Vec<&MarketWatchCard> = cards
        .iter()
        .filter(|card| card.observed_frequency == "monthly")
        .collect();
    let annual: Vec<&MarketWatchCard> = cards
        .iter()
        .filter(|card| card.observed_frequency == "annual")
        .collect();
    let other: Vec<&MarketWatchCard> = cards
        .iter()
        .filter(|card| {
            !matches!(
                card.observed_frequency.as_str(),
                "weekly" | "monthly" | "annual"
            )
        })
        .collect();
    let three = !weekly.is_empty()
        && !monthly.is_empty()
        && !annual.is_empty()
        && other.is_empty();
    let mut parts = Vec::new();
    if !weekly.is_empty() {
        parts.push(format_named_clocks(&weekly));
    }
    if !monthly.is_empty() {
        parts.push(format_grouped_clock(&monthly, "monthly"));
    }
    if !annual.is_empty() {
        parts.push(format_named_clocks(&annual));
    }
    if !other.is_empty() {
        parts.push(format_grouped_clock(&other, "other"));
    }
    let prefix = if three {
        "Three clocks on this page. "
    } else {
        "Clocks on this page. "
    };
    format!("{prefix}{}", parts.join(" "))
}

fn format_named_clocks(cards: &[&MarketWatchCard]) -> String {
    cards
        .iter()
        .map(|card| {
            let current = match card.days_old {
                Some(days) if days <= card.cadence_days => ", current",
                _ => "",
            };
            format!(
                "{} is {}, {}{current}.",
                card.series_id, card.geography, card.observed_frequency
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_grouped_clock(cards: &[&MarketWatchCard], frequency: &str) -> String {
    let ids: Vec<&str> = cards.iter().map(|card| card.series_id.as_str()).collect();
    let mut geos: Vec<&str> = cards.iter().map(|card| card.geography.as_str()).collect();
    geos.sort_unstable();
    geos.dedup();
    let geo_phrase = geos.join(" or ");
    let n = cards.len();
    let verb = if n == 1 { "is" } else { "are" };
    format!("{n} series {verb} {geo_phrase} {frequency} ({}).", ids.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rank::tests::{test_bands, test_config};
    use crate::series::add_days;
    use radar_config::SeriesEntry;
    use rusqlite::params;

    fn open() -> Connection {
        let mut conn = Connection::open_in_memory().expect("memory");
        crate::migrate::migrate(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO load_runs (id, source_id, status, finished_at)
             VALUES (1, 'src-fred', 'success', '2026-09-05')",
            [],
        )
        .expect("run");
        conn
    }

    fn config_with(series: Vec<SeriesEntry>) -> radar_config::RadarConfig {
        let mut config = test_config(1, "caveat", test_bands());
        config.series.series = series;
        config
    }

    fn entry(id: &str, label: &str, geography: &str, included: bool) -> SeriesEntry {
        SeriesEntry {
            series_id: id.to_string(),
            label: label.to_string(),
            geography: geography.to_string(),
            included,
            notes: None,
        }
    }

    fn insert_obs(conn: &Connection, series_id: &str, date: &str, value: Option<f64>) {
        conn.execute(
            "INSERT INTO series_observations (
                 series_id, observation_date, value, realtime_start, realtime_end,
                 load_run_id, fetched_at
             ) VALUES (?1, ?2, ?3, '2026-09-05', '9999-12-31', 1, '2026-09-05T00:00:00+00:00')",
            params![series_id, date, value],
        )
        .expect("obs");
    }

    fn insert_catalog(conn: &Connection, series_id: &str, frequency: &str, geography: &str) {
        conn.execute(
            "INSERT INTO series_catalog (
                 series_id, title, units, frequency, seasonal_adjustment, geography,
                 publication_lag_days, lag_measured
             ) VALUES (?1, ?1, 'Units', ?2, 'NSA', ?3, NULL, 0)",
            params![series_id, frequency, geography],
        )
        .expect("catalog");
    }

    fn days_ago(days: i64) -> String {
        add_days(&today_ymd(), -days).expect("date")
    }

    #[test]
    fn lists_included_series_from_config_not_a_hardcoded_set() {
        let conn = open();
        insert_catalog(&conn, "KEEP", "Monthly", "national");
        insert_catalog(&conn, "DROP", "Monthly", "national");
        insert_obs(&conn, "KEEP", &days_ago(10), Some(1.0));
        insert_obs(&conn, "KEEP", &days_ago(40), Some(2.0));
        insert_obs(&conn, "DROP", &days_ago(10), Some(9.0));
        insert_obs(&conn, "DROP", &days_ago(40), Some(8.0));
        let page = list_market_watch(
            &config_with(vec![
                entry("KEEP", "Kept series", "national", true),
                entry("DROP", "Dropped series", "national", false),
            ]),
            &conn,
        )
        .expect("page");
        assert_eq!(page.cards.len(), 1);
        assert_eq!(page.cards[0].series_id, "KEEP");
        assert!(page.grain_note.contains("do not join to parcels"));
        assert!(page.grain_note.contains("not lead signals"));
    }

    #[test]
    fn sorts_county_then_state_then_national() {
        let conn = open();
        for (id, geo, freq) in [
            ("NAT", "national", "Monthly"),
            ("CO", "county", "Annual"),
            ("ST", "state", "Monthly"),
        ] {
            insert_catalog(&conn, id, freq, geo);
            insert_obs(&conn, id, &days_ago(20), Some(1.0));
            insert_obs(&conn, id, &days_ago(50), Some(2.0));
        }
        insert_obs(&conn, "CO", &days_ago(400), Some(3.0));
        let page = list_market_watch(
            &config_with(vec![
                entry("NAT", "National", "national", true),
                entry("ST", "State", "state", true),
                entry("CO", "County", "county", true),
            ]),
            &conn,
        )
        .expect("page");
        let ids: Vec<&str> = page.cards.iter().map(|c| c.series_id.as_str()).collect();
        assert_eq!(ids, vec!["CO", "ST", "NAT"]);
    }

    #[test]
    fn stale_when_older_than_twice_cadence() {
        let conn = open();
        insert_catalog(&conn, "HOUST", "Monthly", "national");
        insert_obs(&conn, "HOUST", &days_ago(100), Some(12.0));
        insert_obs(&conn, "HOUST", &days_ago(130), Some(11.0));
        let page = list_market_watch(
            &config_with(vec![entry("HOUST", "Housing starts", "national", true)]),
            &conn,
        )
        .expect("page");
        assert_eq!(page.cards[0].cadence_days, 30);
        assert_eq!(page.cards[0].days_old, Some(100));
        assert!(page.cards[0].stale);
        let text = page.cards[0].stale_text.as_deref().unwrap();
        assert!(text.contains("100 days old"));
        assert!(text.contains("twice"));
    }

    #[test]
    fn not_stale_within_twice_cadence() {
        let conn = open();
        insert_catalog(&conn, "HOUST", "Monthly", "national");
        insert_obs(&conn, "HOUST", &days_ago(40), Some(12.0));
        insert_obs(&conn, "HOUST", &days_ago(70), Some(11.0));
        let page = list_market_watch(
            &config_with(vec![entry("HOUST", "Housing starts", "national", true)]),
            &conn,
        )
        .expect("page");
        assert_eq!(page.cards[0].days_old, Some(40));
        assert!(!page.cards[0].stale);
        assert!(page.cards[0].stale_text.is_none());
    }

    #[test]
    fn recent_table_caps_at_24_and_keeps_null_as_none() {
        let conn = open();
        insert_catalog(&conn, "HOUST", "Monthly", "national");
        for i in 0..30 {
            let value = if i == 0 { None } else { Some(i as f64) };
            insert_obs(&conn, "HOUST", &days_ago(i * 30), value);
        }
        let page = list_market_watch(
            &config_with(vec![entry("HOUST", "Housing starts", "national", true)]),
            &conn,
        )
        .expect("page");
        assert_eq!(page.cards[0].recent.len(), 24);
        assert!(page.cards[0].latest_value.is_none());
        assert_ne!(page.cards[0].latest_value, Some(0.0));
    }

    #[test]
    fn county_history_sentence_is_computed_from_the_date() {
        let conn = open();
        insert_catalog(&conn, COUNTY_SERIES_ID, "Annual", "county");
        insert_obs(&conn, COUNTY_SERIES_ID, "2025-01-01", Some(2634.0));
        insert_obs(&conn, COUNTY_SERIES_ID, "2024-01-01", Some(2500.0));
        let page = list_market_watch(
            &config_with(vec![entry(
                COUNTY_SERIES_ID,
                "Private housing units authorised",
                "county",
                true,
            )]),
            &conn,
        )
        .expect("page");
        let card = &page.cards[0];
        let days = card.days_old.expect("days");
        let text = card.history_text.as_deref().expect("history");
        assert!(text.contains("2025"), "{text}");
        assert!(text.contains("Waukesha"), "{text}");
        assert!(text.contains(&days.to_string()), "{text}");
        assert!(text.contains("history"), "{text}");
        assert!(text.contains("not a current reading"), "{text}");
        if days > 600 {
            assert!(text.contains("over 600"), "{text}");
        }
        assert!(!text.contains("percentile"));
        assert_eq!(card.observed_frequency, "annual");
        assert_eq!(card.stale, days > 365 * 2);
    }
}
