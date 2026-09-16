# ADR-0001: Configuration loaded at runtime, not compiled into migrations

Status: Accepted
Date: 30 August 2026
Decision owner: Zubia Mughal

## Context

DrData_ICP_Lead_Radar is the second vertical built on the Decision
Desk approach and the first intended to be sold as a configurable
product. There are already requests for additional verticals.

The first vertical, Karl Intel, compiles series and concept
configuration into migrations using include_str!, and frontend copy
branches on hardcoded concept keys. Adding a vertical to that
codebase requires a migration and frontend edits rather than a
configuration change.

The following are per-vertical and none of them are code:
vertical selection, entity universe, source family, the attribute set
with a direction per attribute, concepts and weights, bucket rules,
classifier and outreach vocabulary lists, capacity, and zone
assignments per capability.

Attribute direction is the most dangerous of these. The same series
reads in opposite directions for two trades. Median listing price
rising is favourable for a realtor and unfavourable for a business
selling downstream of a completed sale. Direction stored as a
constant in code will be wrong for the second vertical and the error
will be silent.

## Decision

All per-vertical configuration loads from runtime configuration files
rather than compiling into migrations or branching in the frontend.

A loader, a schema, and a validator are written before any scoring or
UI code.

## Rejected alternatives

Rejected: the Karl Intel include_str! pattern. Rejected because
adding a vertical requires a migration and frontend edits, and
because a shipped product cannot require a rebuild to be resold.

Rejected: hardcoding this vertical now and extracting configuration
later. Rejected because rebuilding after a sale is more expensive
than building it correctly once, and because requests for additional
verticals already exist.

## Reference implementation to evaluate

MVP_Hub loads nine TOML files at runtime. MVP_Hub has no adapters, no
UI and no scoring engine, so it is not a source of components. The
config loading approach is the one element worth lifting rather than
reinventing. It is to be reviewed before code is written, not assumed
suitable.

## Accepted cost

Build one is slower. A loader, a validator and a schema are written
before a single prospect name appears on screen. The first list does
not arrive this week.

## Consequences

Attribute direction is stored as data, per indicator, per vertical.
It is never a constant in code.

Adding a vertical is a configuration file and an elicitation
conversation, not a migration.
