-- Parcel key is county_fips + muni_code + taxkey.
-- county_fips is already on entities. Nulls are backfilled from
-- load_runs.source_id, which is the spine_source_id written from
-- entity.toml county config at fetch time.
--
-- Before this migration, entities had no UNIQUE constraint. Uniqueness
-- was assertion B on taxkey alone and current_entities resolved one
-- row per taxkey. That dropped one of two real Milwaukee parcels that
-- share a printed taxkey across RecSource systems.

UPDATE entities
SET county_fips = (
    SELECT CASE load_runs.source_id
        WHEN 'src-waukesha-parcels' THEN '55133'
        WHEN 'src-milwaukee-parcels' THEN '55079'
        ELSE entities.county_fips
    END
    FROM load_runs
    WHERE load_runs.id = entities.load_run_id
)
WHERE county_fips IS NULL;

ALTER TABLE entities ADD COLUMN rec_source TEXT;

DROP VIEW current_entities;

CREATE VIEW current_entities AS
SELECT e.*
FROM entities e
INNER JOIN load_runs r ON r.id = e.load_run_id
WHERE r.status = 'success'
  AND e.id = (
      SELECT e2.id
      FROM entities e2
      INNER JOIN load_runs r2 ON r2.id = e2.load_run_id
      WHERE e2.taxkey = e.taxkey
        AND e2.county_fips IS e.county_fips
        AND e2.muni_code IS e.muni_code
        AND r2.status = 'success'
      ORDER BY r2.finished_at DESC, r2.id DESC, e2.id DESC
      LIMIT 1
  );

-- History keeps one row per load run. The unique key is therefore
-- load_run_id plus the parcel key.
CREATE UNIQUE INDEX entities_run_county_muni_taxkey
ON entities (load_run_id, county_fips, muni_code, taxkey);
