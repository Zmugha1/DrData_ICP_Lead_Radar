# ADR-0003: Enrichment provenance

Status: Accepted
Date: 31 August 2026
Decision owner: Zubia Mughal

## Context

Nothing in a parcel roll carries an industry, an employee count, a
website, or whether one person's judgment does the work. Those fields
fill from fetched pages and purchased registries. Values of very
different reliability will sit in adjacent columns: a parcel count
computed from a county record, a category read off a company's own
about page, and a tier assigned by a local model reading that page.
Rendered identically they would be indistinguishable.

## Decision

Every enriched value carries four fields beside it: source,
fetched_at, method, and confidence. Method is an enum with exactly
three values. Fetched, meaning read from a document or record at a
named URL on a named date. Stated, meaning the subject asserts it
about itself. Inferred, meaning a model or a rule produced it.

An Inferred value may be displayed and may be filtered on. It may
never enter a rank, a band, or a score.

Enrichment is insert-only. A re-fetch writes a new row. A prior value
is never overwritten, because a company removing an estimator posting
is itself a signal and is only visible if the old value survives.

## Rejected alternatives

Rejected: a single value column per field with the source in a note.
Rejected because a confirmation flag must be checked everywhere the
value is used and not only where it is displayed. In the first
vertical an unconfirmed figure was gated on export and still carried
fifteen percent of a ranking weight.

## Consequences

One enrichment table serves plats dates, DFI fields, NPI taxonomies
and page extraction without a schema change per source. A validator
rule enforces that no Inferred value reaches ranking.
