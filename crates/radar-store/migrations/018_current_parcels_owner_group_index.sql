-- Serves Owner groups: scan current_parcels by county, then group.
-- The list command only needs owner_name, muni_code, improvement_value,
-- and value_as_of after the county equality.

CREATE INDEX current_parcels_county_owner_group
ON current_parcels (county_fips, owner_name, muni_code, improvement_value, value_as_of);
