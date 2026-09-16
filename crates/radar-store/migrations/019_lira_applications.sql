-- Milwaukee LIRA application-search exports. Insert only.
-- Files are CSVs a human already exported from Application Search
-- (ReportServletLocal format=CSV), one per license type, dropped in
-- C:\Users\zumah\Desktop\radar_sources\milwaukee_lira\. This loader
-- does not fetch itmdapps.milwaukee.gov. It does not store PDF
-- attachments or owner/agent/partner fields.

CREATE TABLE lira_applications (
    source_file TEXT NOT NULL,
    row_index INTEGER NOT NULL,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    fetched_at TEXT NOT NULL,
    license_title TEXT,
    business_name TEXT,
    address TEXT,
    license_type_column TEXT,
    application_date TEXT,
    has_attachment INTEGER,
    PRIMARY KEY (source_file, row_index)
);

CREATE INDEX lira_applications_type
ON lira_applications (license_type_column);

-- Kevin's list: anything that is not a renewal. APPLICATION and
-- each Transfer subtype stay distinct rows; they are not merged.
CREATE VIEW lira_non_renewal_applications AS
SELECT
    source_file,
    row_index,
    load_run_id,
    fetched_at,
    license_title,
    business_name,
    address,
    license_type_column,
    application_date,
    has_attachment
FROM lira_applications
WHERE ifnull(license_type_column, '') != 'RENEWAL';
