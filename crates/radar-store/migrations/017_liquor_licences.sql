-- Milwaukee liquor licences. Insert only. The nightly file is the
-- whole active set, about 1,200 rows, stored again each night so a
-- later snapshot can be compared to an earlier one.

CREATE TABLE liquor_snapshots (
    snapshot_date TEXT NOT NULL PRIMARY KEY,
    source_file TEXT NOT NULL,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    row_count INTEGER NOT NULL,
    fetched_at TEXT NOT NULL
);

CREATE TABLE liquor_licences (
    snapshot_date TEXT NOT NULL REFERENCES liquor_snapshots (snapshot_date),
    row_index INTEGER NOT NULL,
    corp_name TEXT,
    trade_name TEXT,
    licensee TEXT,
    licensee_suffix TEXT,
    address TEXT,
    house_nr TEXT,
    sdir TEXT,
    street TEXT,
    sttype TEXT,
    aldermanic_district TEXT,
    police_district TEXT,
    lic_type TEXT,
    licence_type_full TEXT,
    tot_cap TEXT,
    room_cap TEXT,
    taxkey TEXT,
    eff_date TEXT,
    exp_date TEXT,
    PRIMARY KEY (snapshot_date, row_index)
);

CREATE INDEX liquor_licences_snapshot_key
ON liquor_licences (snapshot_date, taxkey, lic_type);
