-- Kevin's list is Type not RENEWAL. NULL Type is unmapped, not a
-- non-renewal. The 1856 rows from load_run_id 25 stay in
-- lira_applications (insert-only). They drop out of this view.

DROP VIEW IF EXISTS lira_non_renewal_applications;

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
WHERE license_type_column IS NOT NULL
  AND license_type_column != 'RENEWAL';
