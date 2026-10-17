-- Re-verified 8 October 2026 against current_parcels for both
-- counties (55079 Milwaukee, 55133 Waukesha). Insert-only. Does not
-- UPDATE vertical_catalog_elicited. Distinct groups are
-- normalise_owner_name of current_parcels.owner_name.

CREATE TABLE vertical_catalog_county_counts (
    measured_on TEXT NOT NULL,
    vertical_key TEXT NOT NULL REFERENCES vertical_catalog (vertical_key),
    group_count INTEGER NOT NULL,
    method_note TEXT NOT NULL,
    PRIMARY KEY (measured_on, vertical_key)
);

INSERT INTO vertical_catalog_county_counts (
    measured_on, vertical_key, group_count, method_note
) VALUES
('2026-10-08', 'custom_homebuilder', 136,
 'Phrase match on normalised owner_name: CONSTRUCTION, CONTRACTOR, CONTRACTING, BUILDER, BUILDERS, HOMEBUILDER, HOME BUILDER, HOME BUILDING, or HOMEBUILD. Both counties. 376678 distinct normalised names on 447852 named parcel rows.'),
('2026-10-08', 'property_management_cre', 918,
 'Phrase match: PROPERTY MANAGEMENT, PROPERTY MGMT, PROPERTY MGM, PORTFOLIO, REALTY, APARTMENT, APARTMENTS. Both counties. This is name language, not confirmed units under management.'),
('2026-10-08', 'medical_dental_veterinary', 50,
 'Phrase match: MEDICAL, DENTAL, VETERINARY, CLINIC. Both counties. Parcel-visible operators only.'),
('2026-10-08', 'municipal_public_sector', 483,
 'Government-form match: CITY OF, VILLAGE OF, TOWN OF, COUNTY OF, SCHOOL DISTRICT, or name starts with WAUKESHA COUNTY or MILWAUKEE COUNTY. Loose token CITY/VILLAGE/TOWN/COUNTY was 761 and was rejected as matching holding LLCs.'),
('2026-10-08', 'manufacturer_custom_fabricator', 3,
 'Named strings from the ICP document: JW WINCO, JONES INDUSTRIES, GREEN S MACHINES / GREEN and MACHINE. Hits: JW WINCO INC AND MANUFACTURING DIVISION, JONES INDUSTRIES INC, GREEN S MACHINES PROPERTIES LLC.');
