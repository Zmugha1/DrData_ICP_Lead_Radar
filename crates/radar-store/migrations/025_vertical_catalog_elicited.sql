-- Eight verticals named in the ICP document that 005 did not seed.
-- Insert-only: existing vertical_catalog rows are not updated.
-- Elicited descriptions and confirmed-in-county counts live here,
-- not on the 024 paraphrase table, so framework rows can be null
-- rather than invented.

INSERT INTO vertical_catalog (vertical_key, label, tier) VALUES
('medical_dental_veterinary', 'Medical, dental and veterinary', '2'),
('behavioral_health', 'Behavioral health', '2'),
('municipal_public_sector', 'Municipal and public sector', '2'),
('accounting_bookkeeping', 'Accounting and bookkeeping', '2'),
('law_firm_repeatable_practice', 'Law firm, repeatable practice', '2'),
('financial_advisor_insurance', 'Financial advisor and insurance', '2'),
('wholesale_distribution', 'Wholesale and distribution', '1'),
('staffing_recruiting', 'Staffing and recruiting', '1');

CREATE TABLE vertical_catalog_elicited (
    vertical_key TEXT PRIMARY KEY REFERENCES vertical_catalog (vertical_key),
    description TEXT,
    confirmed_in_county_count INTEGER
);

INSERT INTO vertical_catalog_elicited (
    vertical_key, description, confirmed_in_county_count
) VALUES
('custom_homebuilder',
 'Guaranteed or fixed pricing on long build cycles. Six to twelve months between signing a price and buying the materials. Margin erosion nobody measures because it is absorbed into cost of goods before it reaches a statement.',
 147),
('specialty_trade',
 'Electrical, mechanical, plumbing, fire protection, low voltage. Takeoff from builder plans, priced against a rate card somebody updates once a year.',
 NULL),
('landscape_design_build',
 'Estimating from plans, seasonal, heavy material cost exposure, one designer whose eye sells the job.',
 NULL),
('restoration',
 'Carrier relationships and claim history are the entire asset. Xactimate history nobody analyzes.',
 NULL),
('manufacturer_custom_fabricator',
 'Quoting from customer drawings. One estimator who knows the machines, the material yields, and the customers. Frequently near retirement.',
 3),
('property_management_cre',
 'This is the biggest single business type in the county data and it splits in two. An operator with a portfolio, a maintenance budget and renewal pricing decisions is a real prospect. A single-property LLC formed to hold one building, no employees, no decisions, is not. Most of the entities in this category will be the second kind.',
 949),
('realtor', NULL, NULL),
('title_mortgage_insurance', NULL, NULL),
('msp_it_services', NULL, NULL),
('pest_control', NULL, NULL),
('energy_assessor',
 'The originating vertical. Judgment on which homeowner leads convert, priced against the client''s own job history.',
 NULL),
('medical_dental_veterinary',
 'HIPAA makes the argument for you. Scheduling patterns, treatment planning, referral sources.',
 51),
('behavioral_health',
 'Highest sensitivity, strongest custody argument.',
 NULL),
('municipal_public_sector',
 'Records requirements, public accountability, audit expectations.',
 482),
('accounting_bookkeeping',
 'Client financials. The partner''s judgment about which clients are at risk, which engagements earn, which need pricing changes.',
 NULL),
('law_firm_repeatable_practice',
 'Estate planning, real estate closings, family law, workers compensation. Precedent and past matters. One attorney''s method.',
 NULL),
('financial_advisor_insurance',
 'Client data, compliance obligations, a book built over twenty years.',
 NULL),
('wholesale_distribution',
 'Pricing tiers, margin by SKU, customer buying patterns. Rep discretion on pricing with no visibility into which discretion earns.',
 NULL),
('staffing_recruiting',
 'Placement history and who actually works out. Everything in a recruiter''s head.',
 NULL);
