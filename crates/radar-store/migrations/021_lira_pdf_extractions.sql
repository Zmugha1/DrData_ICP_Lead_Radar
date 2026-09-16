-- Insert-only extraction of LIRA application PDFs.
-- Identity is the application attachment, not an owner group_key.
-- ADR-0006 is not used here. See ADR-0010.

CREATE TABLE lira_pdf_extractions (
    id INTEGER PRIMARY KEY,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    application_id TEXT NOT NULL,
    application_source_file TEXT NOT NULL,
    source_url TEXT,
    model_id TEXT NOT NULL,
    extracted_at TEXT NOT NULL,
    trade_name TEXT,
    legal_entity_name TEXT,
    business_address TEXT,
    business_phone TEXT,
    contact_email TEXT,
    email_is_free_provider INTEGER,
    email_domain TEXT,
    license_type TEXT,
    service_style TEXT,
    sells_regulated_goods INTEGER,
    sound_amplification TEXT,
    sound_amplification_desc TEXT,
    security_cameras TEXT,
    security_camera_count INTEGER,
    security_camera_location TEXT,
    building_owner TEXT,
    building_owner_phone TEXT,
    applicant_is_owner TEXT,
    fields_unreadable TEXT NOT NULL,
    CHECK (
        service_style IS NULL
        OR service_style IN (
            'full_service', 'banquet', 'caterer', 'retail',
            'tavern', 'other', 'unknown'
        )
    ),
    CHECK (
        sound_amplification IS NULL
        OR sound_amplification IN ('yes', 'no', 'unknown')
    ),
    CHECK (
        security_cameras IS NULL
        OR security_cameras IN ('yes', 'no', 'unknown')
    ),
    CHECK (
        applicant_is_owner IS NULL
        OR applicant_is_owner IN ('yes', 'no', 'unknown')
    ),
    CHECK (
        email_is_free_provider IS NULL
        OR email_is_free_provider IN (0, 1)
    ),
    CHECK (
        sells_regulated_goods IS NULL
        OR sells_regulated_goods IN (0, 1)
    )
);

CREATE TABLE lira_pdf_extraction_fields (
    extraction_id INTEGER NOT NULL REFERENCES lira_pdf_extractions (id),
    field_name TEXT NOT NULL,
    value_text TEXT,
    page_number INTEGER,
    confidence REAL,
    unreadable INTEGER NOT NULL,
    PRIMARY KEY (extraction_id, field_name),
    CHECK (unreadable IN (0, 1)),
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1))
);
