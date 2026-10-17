-- Snapshot of each Accela occupancy pull. The original accela_occupancy
-- table stays insert-only on record_number. A corrected rerun lands here
-- under a new load_run_id so previously stored rows are not rewritten.

CREATE TABLE accela_occupancy_loads (
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    record_number TEXT NOT NULL,
    source_id TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    value_source TEXT NOT NULL
        CHECK (value_source IN ('csv_grid', 'detail_page')),
    detail_retry INTEGER NOT NULL DEFAULT 0,
    record_date TEXT,
    record_type TEXT,
    status TEXT,
    project_name TEXT,
    site_address TEXT,
    short_notes TEXT,
    cap_id1 TEXT,
    cap_id2 TEXT,
    cap_id3 TEXT,
    applicant_name TEXT,
    work_phone TEXT,
    unlabeled_phone TEXT,
    unlabeled_phone_review INTEGER NOT NULL DEFAULT 0,
    owner_name TEXT,
    owner_mailing_address TEXT,
    legal_business_name TEXT,
    dba TEXT,
    reason_for_applying TEXT,
    business_type TEXT,
    hours TEXT,
    serving_food TEXT,
    serving_alcohol TEXT,
    parking TEXT,
    prior_use TEXT,
    vacancy_duration TEXT,
    taxkey TEXT,
    parcel_matched INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (load_run_id, record_number)
);

CREATE INDEX accela_occupancy_loads_date
ON accela_occupancy_loads (record_date);

CREATE INDEX accela_occupancy_loads_reason
ON accela_occupancy_loads (reason_for_applying);
