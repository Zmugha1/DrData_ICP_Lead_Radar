# ADR-0008: Current parcel state is materialised at fetch, not derived at read

Status: Accepted
Date: 4 September 2026
Decision owner: Zubia Mughal

## Context

The store is insert-only. Every fetch writes a new load run and
nothing is updated in place. current_entities resolves one row per
county, municipality and taxkey from the most recent successful run
containing that key.

That view was a correlated subquery until migration 009 replaced it
with a window function, which fixed a hang. It did not make the reads
fast.

Measured on 4 September 2026:

entities holds 1,757,830 rows across 16 load runs
1,477,156 of them are Waukesha, across 10 successful runs of
142,000 to 163,000 rows each
current Waukesha is 167,456 rows
the database is 756 MiB
populating one dropdown takes 20 to 40 seconds
loading the first page of owner groups takes 21 to 69 seconds

Migration 010 added indexes on the composite key, on load_run_id, on
load run status and on the detections key. The query plans improved,
full table scans became index searches, and the wall clock did not
drop. Two of the timed queries got slower.

The reason is that the view ranks every historical row for a county
to keep one per key. An index locates those 1.48 million rows faster.
It cannot avoid reading them. Every page load pays for all history.

Five of the ten Waukesha runs are identical by construction. Runs 10,
11, 12, 14 and 16 were each executed to prove that a configuration
change, a command line argument and a key change had altered nothing,
and each matched its predecessor on every metric. That was the right
discipline and it left five identical copies of one roll in the
table.

After five further fetches of each county the table would hold
roughly 3,978,170 rows.

## Decision

Current parcel state is materialised into its own table, written at
fetch time inside the same transaction that writes the load run.

One row per county_fips, muni_code, taxkey.
Carrying the same columns the view exposes, plus the load run id
and the value_as_of the row came from.
Replaced within the fetch transaction for keys present in the new
run. A key absent from the new run keeps its existing row and the
value_as_of it already carried, which is what the view does today.
Rebuildable from entities at any time by a command, so it is a
cache of a derivable fact rather than a second source of truth.

entities is unchanged. Nothing is deleted. Every past run stays
readable and any past reading stays reconstructable.

Reads move from 1.76 million rows and growing to roughly 448,000 and
stable.

## Rejected alternatives

Rejected: pruning old load runs. It would make reads fast and it
destroys the ability to reconstruct a past reading, which the
insert-only rule exists to protect. It would also have removed the
five proving runs, which are the evidence that three refactors
changed nothing.

Rejected: resolving current as the most recent successful run only,
with no fallback to earlier runs. Simple and fast, and it changes the
meaning of current. A parcel present in an earlier run and absent
from the newest would disappear rather than resolving with its last
known value and vintage. That behaviour is relied on today: 4,062
Waukesha rows currently resolve from earlier runs.

Rejected: splitting the store by county. Waukesha alone holds 1.48
million rows, so it does not address the cause.

Rejected: further indexing. Migration 010 demonstrated that the
planner was not the constraint.

## Consequences

A fetch does more work. The materialisation happens inside the fetch
transaction, so a failed materialisation fails the run and writes
nothing, consistent with every existing assertion.

There are now two representations of the same fact. The table is
authoritative for reads and entities remains authoritative for
history. A rebuild command must exist from the start, and a check
that the materialised table agrees with the view should run after
every fetch and be reported.

Grouping still happens in Rust after pulling every current row into
memory. This decision reduces the rows pulled. It does not change
where the grouping happens, and that remains an open question.

Two of the indexes added in migration 010 serve no measurable query
and cost file size. Whether to drop them is a separate decision and
is not made here.

## Open question, not decided

Whether identical proving runs should be recorded differently from
substantive fetches, so that history carries what changed rather than
how many times it was verified. Not decided.
