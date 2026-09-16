-- Existing muni_lookup rows are Waukesha civil divisions.
-- Milwaukee uses a different code system; the table must be
-- scoped by county so E1 and E2 do not mix them.

CREATE TABLE muni_lookup_county (
    county_fips TEXT NOT NULL CHECK (length(county_fips) = 5),
    muni_code TEXT NOT NULL,
    muni_name TEXT,
    source TEXT,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    PRIMARY KEY (county_fips, muni_code)
);

INSERT INTO muni_lookup_county (
    county_fips, muni_code, muni_name, source, load_run_id
)
SELECT '55133', muni_code, muni_name, source, load_run_id
FROM muni_lookup;

DROP TABLE muni_lookup;

ALTER TABLE muni_lookup_county RENAME TO muni_lookup;
