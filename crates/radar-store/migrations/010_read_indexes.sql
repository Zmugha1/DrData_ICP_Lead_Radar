-- Indexes only. No table, data, or view change.
--
-- entities (county_fips, muni_code, taxkey)
-- Serves current_entities WHERE county_fips = ? and the detector join
--   ON l.county_fips IS e.county_fips AND l.muni_code IS e.muni_code
--   AND l.taxkey = e.taxkey
--   (when load_run_id is not the leading equality).
-- The unique index from 004 leads with load_run_id, so it cannot
-- satisfy a county-first read.
--
-- entities (load_run_id)
-- Serves SELECT ... FROM entities WHERE load_run_id = ?1 in fetch
-- post-write checks, compare_load_runs, and detector pair scans.
-- IF NOT EXISTS: import_karl already created this name on the live
-- store; a fresh migrate() path did not.
--
-- load_runs (status, finished_at)
-- Serves current_entities: WHERE r.status = 'success' and
-- ROW_NUMBER ORDER BY r.finished_at DESC, r.id DESC.
--
-- detections (signal_key, earlier_load_run_id, later_load_run_id)
-- Serves newest_set_for_pair:
--   SELECT MAX(detected_at) FROM detections
--   WHERE signal_key = ?1 AND earlier_load_run_id = ?2
--     AND later_load_run_id = ?3
-- detections_newest_set already leads with these three and then
-- detected_at. This index is the exact equality prefix of that query.
--
-- entities (taxkey, county_fips, muni_code)
-- Further index from EXPLAIN QUERY PLAN of current_entities after 009:
--   SCAN e USING INDEX entities_taxkey
-- The window is PARTITION BY e.taxkey, e.county_fips, e.muni_code.
-- entities_taxkey is taxkey only, so the county/muni parts of the
-- partition still require a table lookup and a temp B-tree.

CREATE INDEX entities_county_muni_taxkey
ON entities (county_fips, muni_code, taxkey);

CREATE INDEX IF NOT EXISTS entities_load_run_id
ON entities (load_run_id);

CREATE INDEX load_runs_status_finished
ON load_runs (status, finished_at);

CREATE INDEX detections_signal_pair
ON detections (signal_key, earlier_load_run_id, later_load_run_id);

CREATE INDEX entities_taxkey_county_muni
ON entities (taxkey, county_fips, muni_code);
