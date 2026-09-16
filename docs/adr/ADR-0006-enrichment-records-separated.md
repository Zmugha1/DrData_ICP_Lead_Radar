# ADR-0006: Enrichment records, separated from entity attributes, with
per-field provenance

Status: Accepted
Date: 31 August 2026
Decision owner: Zubia Mughal
Supersedes: ADR-0005

## Context

The store holds county roll data. The tables are load_runs, entities
and muni_lookup. The view current_entities resolves one row per
taxkey from the most recent successful load_run. Entities are keyed
by taxkey. There is no owner_groups table. A group_key is computed
in memory by normalising owner_name. Nothing on that roll is an
industry, an employee count, a website, or a judgment about whether
an operating business exists.

ADR-0003 already requires that every enriched value carry provenance
and that enrichment be insert-only. ADR-0005 recorded a first shape
for an enrichment table and is superseded by this ADR. Two of its
clauses were wrong: it let insert order plus origin decide
precedence, and it omitted not_yet_enriched as the default
render_state.

Ten entities from the favourable band were looked up by hand on
31 August 2026. Six resolved to an operating company with a website.
One was searched and no website exists, which is consistent with a
single-purpose land-holding entity. One has no site of its own while
a separately named company plausibly run by the same person does,
and that link is not published anywhere joinable. Two were not
searched at all. The remaining 2,273 entities have not been looked
at.

Those five outcomes are all different facts and the current schema
can represent none of them.

Enrichment will arrive from web pages and from manual seeding, on
dates that are not the roll vintage. If those values are written
onto entities they stop being a record of the roll.

## Decision

Enrichment is stored in its own insert-only table, not as columns on
entities and not as columns on a computed owner group. Entities come
from the county roll on an annual vintage. Enrichment comes from web
pages on their own dates. Two cadences, two sources, two tables. A
view resolves the current enrichment value per entity per field. The
entity identity on an enrichment row is the computed group_key. That
key is not a column on entities today.

The unit of storage is one field, not one record per company. Each
row holds: entity group_key, field name, value, evidence quote,
source_url, fetched_at, origin. A company's briefing is a query over
its fields, not a blob.

origin is an enumerated column carrying at least manual and the name
of the adapter that produced the row. A manually seeded row and an
adapter row are distinguishable at query time, permanently.

The view resolves the current value by fetched_at descending, then
by insert order. origin does not confer precedence. It is recorded
for audit and for filtering. A manual row fetched today therefore
supersedes an adapter row fetched last month, which is correct,
because the more recent look at the page is the better evidence.
Nothing is ever deleted or updated in place.

An evidence quote is required on every read field. A value without
the text it was read from cannot be verified and is not stored.
This is what makes local model extraction auditable later.

Counted and read are different reliability classes and never merge
into one number or one sentence. Counted values are computed from
the roll and carry the roll vintage. Read values are read from a
named page and carry their own fetched_at. Any surface showing both
must show which is which.

Unknown is stored explicitly, not represented by an absent row. An
absent row and a row whose value is unknown are different facts. A
field that was looked for and not found is recorded as such.

Each entity carries a render_state governing whether it is scored at
all. Five states, and the first is the default on every entity at
import:

not_yet_enriched
operating_business_identified
operating_business_unconfirmed
no_operating_business_identified
not_a_prospect

not_yet_enriched is the state of 2,273 entities today and is a
stated fact rather than an inference from an empty join. It is the
same principle as an explicit unknown, applied at entity level.

The last three are terminal valid states, not errors and not missing
data. Rows in those states render with a stated reason and are never
assigned a score or a band.

Read fields may carry an optional decay note and a recheck interval.
A fact true on a date is not a fact true today. Association office
and employment status decay annually.

An enrichment row may name a related company in its value or its
evidence and must never join to it. Where a separately named company
is plausibly the same operator, that is recorded as text inside a
field on the entity being enriched, with confidence stated. It never
creates an edge between two group_keys. A link between two entities
requires a published record that names both, and no such record
exists in any source now wired.

## Rejected alternatives

Rejected: adding enrichment columns to the entity table. Rejected
because the source and the cadence are different, and because it
would make the roll table stop being a faithful record of the roll.

Rejected: storing one free-text business description per company.
Rejected because it is not verifiable field by field and cannot
carry per-field provenance. It is also the exact shape that invites
a model to write a plausible paragraph.

Rejected: storing the extracted value without the evidence quote.
Rejected because it is unverifiable, and because it removes the only
check on a local model extraction.

Rejected: treating "no website found" as an error or a retry
condition. Rejected because for a single-purpose land-holding entity
there is no website to find. It is a terminal state and must be
representable.

Rejected: updating enrichment rows in place. Rejected because it
violates the standing insert-only rule and destroys the ability to
reconstruct what a briefing said last month.

Rejected: letting origin decide precedence, so that an adapter row
always beats a manual one. Rejected because it would silently
discard the most recent human reading of a page in favour of an
older automated one.

Rejected: joining an entity to a similarly named operating company
found during lookup. Rejected because the link is an inference.
Encoding it would turn a plausible association into a stated fact,
and the failure would be invisible because the joined row would
render identically to a verified one.

## Constraints

No scoring change is implied by this ADR. Ranking stays as it is.

render_state affects display and scoring eligibility only.

Cloud extraction is permitted for public records, enforced at the
call site. Client data never enters an extraction prompt. That is a
constraint on the future adapter. This ADR does not implement it.

## Consequences

A later migration will add the enrichment table and the current
enrichment view. This ADR does not create that migration.

A briefing is assembled by querying fields for a group_key. Counted
fields continue to come from the roll and from owner-group
computation. Read fields come from the enrichment table. Surfaces
that show both must label the class.

An Inferred value, as defined in ADR-0003, may still never enter a
rank, a band, or a score. render_state is a separate gate:
operating_business_unconfirmed, no_operating_business_identified,
and not_a_prospect receive no score and no band regardless of
method. not_yet_enriched is the default and is not an empty join.
