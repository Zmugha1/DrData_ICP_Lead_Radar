-- Outcomes are a record of actions against a subject. Insert only.
-- A row is never updated or deleted. A mistake is corrected by
-- recording another outcome, and both stay. None is a status on the
-- subject and none is superseded.
--
-- Recording is the whole job until there are months of data. Nothing
-- may read this table to rank, score, convert, predict or suggest.
-- This data will eventually inform the Below Zone assignment
-- prospect_ranking_by_likelihood_to_buy. Until then, capture only.

CREATE TABLE outcomes (
    id INTEGER PRIMARY KEY,
    subject_type TEXT NOT NULL,
    subject_key TEXT NOT NULL,
    county_fips TEXT,
    action TEXT NOT NULL,
    occurred_on TEXT NOT NULL,
    reason TEXT,
    config_fingerprint TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    CHECK (subject_type IN ('owner_group', 'assignment_assignee')),
    CHECK (action IN ('contacted', 'met', 'proposed', 'closed_won', 'closed_lost', 'not_a_fit'))
);

CREATE INDEX outcomes_subject
ON outcomes (subject_type, subject_key, occurred_on, recorded_at, id);
