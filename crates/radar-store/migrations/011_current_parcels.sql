-- Materialised current parcel state. Rebuildable from entities.
-- current_entities stays as the definition of truth.

CREATE TABLE current_parcels (
    county_fips TEXT NOT NULL,
    muni_code TEXT NOT NULL,
    taxkey TEXT NOT NULL,
    id INTEGER NOT NULL,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    owner_name TEXT,
    site_address TEXT,
    mail_address_1 TEXT,
    mail_address_2 TEXT,
    assessed_value REAL,
    improvement_value REAL,
    land_value REAL,
    owner_type TEXT,
    mail_matches_site INTEGER,
    prop_class TEXT,
    value_source TEXT,
    value_as_of TEXT,
    source_layer_url TEXT,
    fetched_at TEXT,
    rec_source TEXT,
    materialised_at TEXT NOT NULL,
    PRIMARY KEY (county_fips, muni_code, taxkey)
);
