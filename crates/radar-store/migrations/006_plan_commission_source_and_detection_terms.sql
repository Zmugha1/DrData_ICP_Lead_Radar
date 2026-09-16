-- plan_commission_item was seeded against contractor rolls. Those are
-- licensed-trade lists, not plan commission packets.

UPDATE signal_catalog
SET source_id = 'src-municipal-plan-commission-agendas'
WHERE signal_key = 'plan_commission_item';

INSERT INTO term_glossary (term_key, label, body, category) VALUES
('source_status', 'Source status', 'Whether the underlying record source is fetching today.', 'signals'),
('detection_built', 'Detection built', 'Whether the code that spots this change exists. A live source with no detection produces nothing.', 'signals');
