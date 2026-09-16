-- Detections are observations. Insert only. A row is never updated.
-- A re-run against the same pair of load runs inserts a new set.
-- The newest set for a pair is every row with that signal_key, that
-- earlier_load_run_id, that later_load_run_id, and detected_at equal
-- to MAX(detected_at) for those three.

CREATE TABLE detections (
    id INTEGER PRIMARY KEY,
    signal_key TEXT NOT NULL,
    detected_at TEXT NOT NULL,
    earlier_load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    later_load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    county_fips TEXT,
    muni_code TEXT,
    taxkey TEXT NOT NULL,
    earlier_value REAL,
    later_value REAL,
    earlier_value_as_of TEXT,
    later_value_as_of TEXT,
    window_days INTEGER,
    config_fingerprint TEXT
);

CREATE INDEX detections_newest_set
ON detections (signal_key, earlier_load_run_id, later_load_run_id, detected_at);

CREATE INDEX detections_county_signal
ON detections (signal_key, county_fips, detected_at);
