# ADR-0007: Two-layer parcel fetch with a coverage ratio fallback

Status: Accepted
Date: 1 September 2026
Decision owner: Zubia Mughal

## Context

The county publishes parcels on a live layer and on year-end
historic layers. PROPCLASS, the property class field, is not a
single code. It holds 204 distinct values, most of them
comma-separated lists such as '1,4' and '4,1'. An exact match on
'1' drops every mixed-use parcel that is partly residential.

Seven municipalities carry thousands of live rows and zero rows
under any PROPCLASS filter. They do not populate the field on the
live layer. Six of the seven are fully classed on the previous
year-end historic layer.

The original data, load_run 1, was assembled by hand from two
layers and two vintages. The rule that produced it was never
written down.

Four runs were needed to recover it:

load_run 2 used exact match on one layer. It returned 78,417 rows
and silently omitted seven municipalities. A one-directional join
check passed because a filter's exclusion never enters the fetched
set.

load_run 3 used LIKE and two layers. It rolled back on six taxkeys
appearing in both passes.

load_run 4 excluded pass 1 taxkeys from pass 2 and committed
131,000 rows. It lost 13,605 rows because the fallback rule tested
for a live count of exactly zero, and Menomonee Falls has 34
classed rows out of 15,693 unfiltered. The rule tested for presence
rather than for coverage.

load_run 5 replaced the zero test with a coverage ratio and
committed 144,623 rows.

## Decision

The parcel spine is fetched in two passes.

Pass 1 fetches the live layer with a substring match on the
property class rather than an exact match, so mixed-use parcels
that are partly residential are included.

Before any rows are fetched, each municipality's coverage ratio is
computed from two counts on the live layer: rows matching the
filter, divided by rows unfiltered. A municipality whose ratio
falls below a configured threshold is fetched from the historic
year-end layer instead.

The threshold lives in configuration as
spine_coverage_fallback_threshold and is validated to be strictly
between zero and one. No municipality code is hardcoded. The
fallback set is computed on every run.

Pass 2 excludes any taxkey already collected in pass 1. Live wins,
because the live layer is seven months newer than the year-end
layer.

A municipality with no classed rows on either layer is recorded as
uncovered. That is a coverage fact, not a run failure.

Every row carries its own value_source and value_as_of, read from
the layer it came from. A single run therefore holds two vintages,
which is what the roll actually is. Run-level columns that cannot
hold two values are left null rather than picking one. Lag is
reported per source: nine days live, 214 days historic.

## Rejected alternatives

Rejected: exact match on the property class. It dropped 1,474
mixed-use parcels and, combined with a one-directional join check,
omitted seven municipalities without failing.

Rejected: a fallback rule testing for a live count of zero.
Rejected because Menomonee Falls sits at 0.002 and passed a
presence test while contributing 13,605 missing rows. Presence is
not coverage.

Rejected: hardcoding the fallback municipality list. Rejected
because a municipality that begins classing its parcels would
continue to be read from a stale layer, silently.

Rejected: deduplicating after assembly. Rejected because the rule
should be stated at collection time rather than cleaned up
afterwards, and because a distinct clause hides the count.

Rejected: a single run-level vintage or lag. Rejected because a run
holding two vintages has two lags, and picking one either hides the
January data or makes the live data look worse than it is.

Rejected: reusing expected_match_rate on the attachment to hold the
threshold. Rejected because that field is a join completeness
figure for the municipality lookup, validated by a different rule,
and is a different quantity.

## Consequences

The threshold is a per-vertical configuration value. A vertical
whose spine has no fallback layer omits the field.

Roughly 45 percent of the roll is read from the year-end layer and
is current to 30 January 2026 rather than to the fetch date. That
belongs on the coverage page, not in a footnote.

The two layers do not agree on municipality assignment for a small
number of parcels. Forty taxkeys returned by a historic
municipality's result set were already present live under a
different municipality code. They are excluded, live wins, and the
disagreement is recorded here rather than reconciled.
