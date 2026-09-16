-- Series data is a different grain from parcels: one number per period
-- for a geography, against one row per parcel. The two do not join
-- and must not be presented as if they do.
--
-- value is nullable. FRED publishes a full stop for a missing
-- observation; that is a missing value, not zero.
-- Insert only.

CREATE TABLE series_observations (
    series_id TEXT NOT NULL,
    observation_date TEXT NOT NULL,
    value REAL,
    realtime_start TEXT NOT NULL,
    realtime_end TEXT NOT NULL,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (series_id, observation_date, realtime_start)
);

CREATE TABLE series_catalog (
    series_id TEXT PRIMARY KEY,
    title TEXT,
    units TEXT,
    frequency TEXT,
    seasonal_adjustment TEXT,
    geography TEXT,
    publication_lag_days INTEGER,
    lag_measured INTEGER NOT NULL,
    notes TEXT
);
