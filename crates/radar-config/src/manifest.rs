use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum EntityUniverse {
    Parcels,
    BusinessRegistrations,
}

// pages is visibility and order only. Which columns, cards, and
// sections a page shows stay fixed in the app. This is not the
// start of a page builder.

pub const KNOWN_PAGE_KEYS: &[&str] = &[
    "status",
    "owner_groups",
    "assignments",
    "market_watch",
    "coverage",
    "sources",
    "how_to_use",
];

pub const REQUIRED_PAGE_KEYS: &[&str] = &["status", "how_to_use"];

pub const PAGE_NOT_ENABLED: &str = "This page is not enabled for this configuration.";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageEntry {
    pub key: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NavPage {
    pub key: String,
    pub href: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: String,
    pub config_version: String,
    pub vertical_id: String,
    pub vertical_display_name: String,
    pub county_fips: String,
    pub county_name: String,
    pub entity_universe: EntityUniverse,
    pub pages: Vec<PageEntry>,
}

pub fn default_pages() -> Vec<PageEntry> {
    KNOWN_PAGE_KEYS
        .iter()
        .map(|key| PageEntry {
            key: (*key).to_string(),
            label: None,
        })
        .collect()
}

pub fn known_page_key(key: &str) -> bool {
    KNOWN_PAGE_KEYS.iter().any(|known| *known == key)
}

pub fn default_page_label(key: &str) -> Option<&'static str> {
    match key {
        "status" => Some("Status"),
        "owner_groups" => Some("Owner groups"),
        "assignments" => Some("Assignments"),
        "market_watch" => Some("Market watch"),
        "coverage" => Some("Coverage"),
        "sources" => Some("Sources"),
        "how_to_use" => Some("How to use this"),
        _ => None,
    }
}

pub fn page_href(key: &str) -> Option<&'static str> {
    match key {
        "status" => Some("index.html"),
        "owner_groups" => Some("groups.html"),
        "assignments" => Some("assignments.html"),
        "market_watch" => Some("market.html"),
        "coverage" => Some("coverage.html"),
        "sources" => Some("sources.html"),
        "how_to_use" => Some("howto.html"),
        _ => None,
    }
}

pub fn nav_pages(manifest: &Manifest) -> Vec<NavPage> {
    manifest
        .pages
        .iter()
        .filter_map(|entry| {
            let href = page_href(&entry.key)?;
            let label = entry
                .label
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .or_else(|| default_page_label(&entry.key).map(str::to_string))?;
            Some(NavPage {
                key: entry.key.clone(),
                href: href.to_string(),
                label,
            })
        })
        .collect()
}

pub fn page_is_enabled(manifest: &Manifest, key: &str) -> bool {
    manifest.pages.iter().any(|page| page.key == key)
}
