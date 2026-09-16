UPDATE signal_catalog
SET description = description || ' This signal compares two fetches. It produces nothing unless the source has republished between them. Waukesha''s live layer carried export dates 2026-08-10 and 2026-08-23, thirteen days apart, so the useful cadence for this signal is however often the county republishes, not however often you fetch. Milwaukee publishes no export date at all, so a Milwaukee pair cannot be dated and its windows are unknown.'
WHERE signal_key = 'improvement_zero_to_value';
