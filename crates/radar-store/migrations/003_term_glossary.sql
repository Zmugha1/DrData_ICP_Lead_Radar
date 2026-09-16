CREATE TABLE term_glossary (
    term_key TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    body TEXT NOT NULL,
    category TEXT NOT NULL
);

INSERT INTO term_glossary (term_key, label, body, category) VALUES
('inside_zone', 'Inside Zone', 'The rule can be stated, its exceptions named, and a recent example given. It is safe to encode.', 'zones'),
('below_zone', 'Below Zone', 'The rule can be stated but has not been shown to predict anything. It is shown as a starting place, never as a prediction.', 'zones'),
('above_zone', 'Above Zone', 'The task is beyond what anyone here can do reliably, so encoding it would turn a guess into an answer. It is excluded.', 'zones'),
('oscillating_zone', 'Oscillating Zone', 'The expert answers differently on different days and the difference is not explained by changed conditions. It is returned to elicitation.', 'zones'),
('taxkey', 'Taxkey', 'The county''s identifier for a parcel.', 'parcel data'),
('improvement_value', 'Improvement value', 'The assessed value of buildings on a parcel, separate from the land. Zero means the county recorded no building value. Blank means the county did not publish a figure, which is not the same thing.', 'parcel data'),
('assessed_value', 'Assessed value', 'The county''s total assessed value.', 'parcel data'),
('land_value', 'Land value', 'The assessed value of the land alone.', 'parcel data'),
('prop_class', 'Property class', 'The county''s property class. In Waukesha it is a comma separated list and one parcel can carry several.', 'parcel data'),
('muni_code', 'Municipality code', 'The municipality code on the parcel record.', 'parcel data'),
('owner_individual', 'Individual owner', 'The deed names a person.', 'owner type'),
('owner_trust', 'Trust owner', 'The deed names a trust, usually a revocable living trust on a family home.', 'owner type'),
('owner_entity', 'Entity owner', 'The deed names a company.', 'owner type'),
('owner_government', 'Government owner', 'The deed names a public body.', 'owner type'),
('band_no_vacant_holdings', 'No vacant holdings', 'This owner holds no parcels recorded with zero improvement value.', 'bands'),
('band_small_holding', 'Small holding', 'One to five parcels with no improvement value.', 'bands'),
('band_parcels_no_improvement_value', 'Parcels with no improvement value', 'Six or more parcels with no improvement value. Often an active builder or developer, and also sometimes farmland, an association outlot, or land the county has not valued.', 'bands'),
('load_run', 'Load run', 'One fetch from a source. Every row records which run it came from.', 'provenance'),
('value_as_of', 'Value as of', 'The date the source published the figure, not the date it was fetched.', 'provenance'),
('value_source', 'Value source', 'Which layer a row came from, such as the live county layer or a previous year end roll.', 'provenance'),
('publication_lag', 'Publication lag', 'The gap between the date a source published a figure and the date it was fetched.', 'provenance'),
('config_fingerprint', 'Config fingerprint', 'A checksum of the settings in use, so a past reading can be reproduced.', 'provenance'),
('coverage_full', 'Full coverage', 'The source covers this completely, as far as is known.', 'coverage'),
('coverage_partial', 'Partial coverage', 'The source covers part of this. The note says which part.', 'coverage'),
('coverage_none', 'No coverage', 'Not covered at all.', 'coverage'),
('uncovered_municipality', 'Uncovered municipality', 'The municipality exists in the lookup but the source returned no rows for it.', 'coverage');
