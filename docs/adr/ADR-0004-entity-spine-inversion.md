# ADR-0004: Entity spine inversion

Status: Proposed
Date: 31 August 2026
Decision owner: Zubia Mughal

## Context

Parcels are the current spine because they are the only universe
available. Triage of the 47 favourable groups found 30 that are
project LLCs, holding companies or subdivision-named entities with
no operating business behind the name. Developers form an entity per
project. That entity has no website, no employees and no judgment to
encode. Fetching a website for a subdivision-named LLC is not a hard
task, it is an impossible one.

The parcel roll names the entity that holds the land, not the
business that makes decisions. Most businesses matching the ideal
customer profile own no property at all, which makes most of Tier 2
and effectively all of Tier 3 invisible to this spine.

## Proposed decision

Wisconsin DFI business registrations become the entity spine. Parcels
demote to an attachment joined by normalised owner name, with the
match rate measured and asserted rather than assumed.

This ADR is Proposed. It is not implemented and no code depends on
it. It is recorded now so the reason is on file before the work
starts.

## Blocking preconditions

None of these are yet met.

The full corporate database must be purchased, because a monthly
delta with no baseline detects nothing on its first run.

The registered agent field must be profiled and a commercial-agent
exclusion list built, because CT Corporation, Registered Agents Inc
and Northwest will dominate the counts and identify agents rather
than owners.

The name join loss must be measured, given that a trading-name join
in the first vertical matched zero of 628 rows.

## Known limitation

DFI holds no LLC member names, no sole proprietors, no tradenames or
DBAs, and no officers. The registered agent is the only published
route from a project entity to a person, and it is Below Zone until
the exclusion list exists.

## Rejected alternatives

None recorded. This ADR is Proposed. Alternatives will be recorded
when the blocking preconditions are met and the decision is accepted
or withdrawn.
