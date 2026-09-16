-- current_entities previously used a correlated subquery per row.
-- Against a store with multiple load runs that scan is effectively
-- quadratic and does not return. The groups page waited on
-- owner_group_filter_options and showed empty dropdowns, no caveat,
-- and no rows. Semantics are unchanged: one row per
-- (taxkey, county_fips, muni_code) from the latest successful load.

DROP VIEW current_entities;

CREATE VIEW current_entities AS
SELECT
    id,
    load_run_id,
    county_fips,
    taxkey,
    muni_code,
    owner_name,
    site_address,
    mail_address_1,
    mail_address_2,
    assessed_value,
    improvement_value,
    land_value,
    owner_type,
    mail_matches_site,
    prop_class,
    value_source,
    value_as_of,
    source_layer_url,
    fetched_at,
    rec_source
FROM (
    SELECT
        e.*,
        ROW_NUMBER() OVER (
            PARTITION BY e.taxkey, e.county_fips, e.muni_code
            ORDER BY r.finished_at DESC, r.id DESC, e.id DESC
        ) AS rn
    FROM entities e
    INNER JOIN load_runs r ON r.id = e.load_run_id
    WHERE r.status = 'success'
)
WHERE rn = 1;
