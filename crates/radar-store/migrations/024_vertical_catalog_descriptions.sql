-- Descriptions for the eleven vertical_catalog rows seeded in 005.
-- Insert-only companion table. The original vertical_catalog rows are
-- not updated. tier stays a TEXT column on vertical_catalog; it is
-- not a SQLite foreign key to config.

CREATE TABLE vertical_catalog_descriptions (
    vertical_key TEXT PRIMARY KEY REFERENCES vertical_catalog (vertical_key),
    description TEXT NOT NULL
);

INSERT INTO vertical_catalog_descriptions (vertical_key, description) VALUES
('custom_homebuilder',
 'A firm that bids custom homes on lots. Signal catalog already treats a new parcel, a cleared lot, a recorded plat, and a plan commission item as favourable, and a building appearing on the roll as unfavourable because another builder got the job. Classifier language is estimator, takeoff, bid coordinator, guaranteed pricing, and family owned since. This is vertical_catalog.tier 1 as defined in tiers.toml tier_key 1.'),
('specialty_trade',
 'Electrical, plumbing, HVAC and similar trades that bid after a builder is under contract. DSPS credentials and municipal contractor rolls were the source notes that previously said Tier 1 trades. Storm events are favourable trade work. This is vertical_catalog.tier 1 as defined in tiers.toml tier_key 1.'),
('landscape_design_build',
 'A design-build landscape firm. The catalog already records improvement_zero_to_value as favourable because a finished house is when the site is ready, and a teardown as unfavourable. New plats and plan commission site plans are when landscape scope is still open. This is vertical_catalog.tier 1 as defined in tiers.toml tier_key 1.'),
('restoration',
 'Insured repair of existing buildings. classifier_positive already includes IICRC. Storm events are the intended trigger; NOAA is still Scoped. New construction and a cleared lot are unfavourable because there is no building left to restore. This is vertical_catalog.tier 1 as defined in tiers.toml tier_key 1.'),
('manufacturer_custom_fabricator',
 'A maker that quotes from drawings rather than a catalog. classifier_positive already includes made to order, custom fabrication, no two the same, job shop, and quote from drawings. USPTO filings are favourable because a class code names a maker investing in a product line. This is vertical_catalog.tier 1 as defined in tiers.toml tier_key 1.'),
('property_management_cre',
 'Commercial property management. A new owner is when management contracts are reviewed. Occupancy new business, a new liquor licence, and a food licence are tenant-opening signals. A parcel spine still misses operators who own no land. This is vertical_catalog.tier 1 as defined in tiers.toml tier_key 1.'),
('realtor',
 'A listing and occupancy conversation, not a bid-and-estimate shop. New parcels, transfers, occupancy, and liquor are already mapped as favourable. ADR-0001 records that median listing price rising is favourable for a realtor. This is vertical_catalog.tier 3 as defined in tiers.toml tier_key 3, which is the configuration ticket rather than a full build.'),
('title_mortgage_insurance',
 'Title, mortgage and insurance work that follows a closing or a loss. New parcels and a different owner are favourable because a closing occurred. A storm is when insurance files open. This is the only store row currently encoded as vertical_catalog.tier 2, matching tiers.toml tier_key 2 custody language (fiduciary) rather than the estimator cluster.'),
('msp_it_services',
 'Network and records systems for a newly occupying business. Occupancy new business and a newly enumerated clinic NPI are already mapped as favourable. classifier_negative already includes we want AI and streamline our processes, which are the walk-away for this vertical. This is vertical_catalog.tier 3 as defined in tiers.toml tier_key 3.'),
('pest_control',
 'Pest service at a food or clinical address. New liquor licences, food licences, occupancy new business, and a new structure are favourable; a closed licence or a teardown ends the account. DATCP notes previously said they serve hospitality and the pest vertical. This is vertical_catalog.tier 3 as defined in tiers.toml tier_key 3.'),
('energy_assessor',
 'Energy assessment of buildings. improvement_zero_to_value is favourable because a new building is a candidate. Vacant-lot ranking in attributes.toml is the documented use, with the energy-client anecdote in zones.toml. This is vertical_catalog.tier 1 as defined in tiers.toml tier_key 1.');
