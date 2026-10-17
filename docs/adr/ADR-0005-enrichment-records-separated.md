# ADR-0005: Enrichment records, separated from entity attributes,
with per-field provenance (superseded)

Status: Superseded by ADR-0006
Date: 31 August 2026
Decision owner: Zubia Mughal

## Context

The store holds county roll data. The tables are load_runs, entities
and muni_lookup. The view current_entities resolves one row per
taxkey from the most recent successful load_run. Entities are keyed
by taxkey. There is no owner_groups table. A group_key is computed
in memory by normalising owner_name. Nothing on that roll is an
industry, an employee count, a website, or a judgment about whether
an operating business exists.

ADR-0003 already requires that every enriched value carry provenance
and that enrichment be insert-only. It does not record the table
shape, the unit of storage, or how a looked-for-and-not-found field
differs from a field that was never asked. Those are this ADR.

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

origin is an enumerated column with at least the values manual and
the adapter name that produced the row. A manually seeded row and an
adapter-produced row are distinguishable at query time, permanently.
An adapter row supersedes a manual row for the same entity and field
by being inserted later. The manual row is never deleted.

evidence quote is required on every read field. A value without the
text it was read from cannot be verified and is not stored. This is
what makes local model extraction auditable later.

Counted and read are different reliability classes and never merge
into one number or one sentence. Counted values are computed from
the roll and carry the roll vintage. Read values are read from a
named page and carry their own fetched_at. Any surface showing both
must show which is which.

Unknown is stored explicitly, not represented by an absent row. An
absent row and a row whose value is unknown are different facts. A
field that was looked for and not found is recorded as such.

Each entity carries a render_state governing whether it is scored at
all. At minimum the states are operating_business_identified,
operating_business_unconfirmed, no_operating_business_identified,
and not_a_prospect. The last three are terminal valid states, not
errors and not missing data. Rows in those states render with a
stated reason and are never assigned a score or a band.

Read fields may carry an optional decay note and a recheck interval.
A fact true on a date is not a fact true today. Association office
and employment status decay annually.

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
rank, a band, or a score. render_state is a separate gate: the last
three states receive no score and no band regardless of method.
