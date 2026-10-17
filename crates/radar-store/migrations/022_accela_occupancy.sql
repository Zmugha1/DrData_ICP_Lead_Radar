-- Milwaukee Accela occupancy certificates. Insert only.
-- Natural key is Record Number. Grid-stage rows may land without a
-- successful detail GET; rich fields stay null and detail_retry is set.

CREATE TABLE accela_occupancy (
    record_number TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
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
    parcel_matched INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX accela_occupancy_date
ON accela_occupancy (record_date);

CREATE INDEX accela_occupancy_reason
ON accela_occupancy (reason_for_applying);

INSERT INTO signal_catalog (signal_key, label, description, source_id, detection_method) VALUES
('occupancy_new_business', 'New occupancy, establish a new business or use',
 'An Accela occupancy record whose Reason for applying is 1-Establish a new business or use. Observed by reading stored occupancy rows after the two-stage Accela pull.',
 'src-milwaukee-accela-occupancy', 'run comparison');

INSERT INTO signal_direction (signal_key, vertical_key, direction, rationale) VALUES
('occupancy_new_business', 'msp_it_services', 'favourable',
 'A new occupancy for a new business is a site that needs network, phones, and records set up from zero.'),
('occupancy_new_business', 'property_management_cre', 'favourable',
 'A new occupancy is a tenant opening in commercial space.'),
('occupancy_new_business', 'realtor', 'favourable',
 'A new occupancy is a commercial occupancy event.'),
('occupancy_new_business', 'pest_control', 'favourable',
 'A new occupied commercial space is a service address.');
