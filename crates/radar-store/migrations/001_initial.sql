-- Excluded columns and the reason they are not imported:
--
-- year_built. In the source it holds 1900 on 961 records with 1898
-- and 1899 empty and 1901 at seven. A placeholder spike. Not
-- imported.
--
-- last_sale_date and last_sale_price. 657 records at 1900-01-02 with
-- neighbouring dates empty. A placeholder spike with no accompanying
-- estimated flag. Not imported.
--
-- height_class. Three distinct values, 10, 20 and 30. A LiDAR height
-- class in feet, not a storey count. Not imported.
--
-- All building detail columns. Present for one municipality only.
--
-- Block group columns. Area statistics that describe areas and not
-- households.

CREATE TABLE load_runs (
    id INTEGER PRIMARY KEY,
    source_id TEXT NOT NULL,
    layer_url TEXT,
    where_clause TEXT,
    value_source TEXT,
    value_as_of TEXT,
    started_at TEXT,
    finished_at TEXT,
    status TEXT NOT NULL,
    records_requested INTEGER,
    records_written INTEGER,
    records_skipped INTEGER,
    error_text TEXT
);

CREATE TABLE entities (
    id INTEGER PRIMARY KEY,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    county_fips TEXT,
    taxkey TEXT NOT NULL,
    muni_code TEXT,
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
    fetched_at TEXT
);

CREATE TABLE muni_lookup (
    muni_code TEXT NOT NULL,
    muni_name TEXT,
    source TEXT,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id)
);

CREATE VIEW current_entities AS
SELECT e.*
FROM entities e
INNER JOIN load_runs r ON r.id = e.load_run_id
WHERE r.status = 'success'
  AND e.id = (
      SELECT e2.id
      FROM entities e2
      INNER JOIN load_runs r2 ON r2.id = e2.load_run_id
      WHERE e2.taxkey = e.taxkey
        AND r2.status = 'success'
      ORDER BY r2.finished_at DESC, r2.id DESC, e2.id DESC
      LIMIT 1
  );
