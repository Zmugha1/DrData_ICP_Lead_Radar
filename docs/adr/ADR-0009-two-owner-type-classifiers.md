# ADR-0009: Two owner-type classifiers

Status: Open
Date: 6 September 2026
Decision owner: Zubia Mughal

## Context

`owner_type` is a word with two definitions in this repository.

Fetch classifies each parcel with `classify_owner_type` in
`crates/radar-store/src/parcels.rs` and writes that value onto
`entities.owner_type`. Materialisation copies it onto
`current_parcels.owner_type`. The rule is hardcoded: government only
when the name starts with CITY, VILLAGE, TOWN, COUNTY or STATE plus
OF, or with UNITED STATES; then trust tokens; then legal-form entity
tokens; otherwise individual.

Owner groups, ranking that reads groups, and the default owner-type
filter classify at query time from `owner_type.toml` via
`OwnerTypeFile::classify`. That rule is config: placeholder, then
government, trust and entity phrase lists, with per-phrase anchors.
Reads currently take this config classifier. They do not use the
stored column.

Measured on 6 September 2026, before the placeholder type and COUNTY
anchor change, the two classifiers disagreed on 2,642 parcel rows
(1,895 in Milwaukee, 747 in Waukesha).

## Problem

A caller that reads `current_parcels.owner_type` or
`entities.owner_type` gets the fetch-time answer. A caller that goes
through the owner-groups path gets the config answer. The same field
name means both. The stored column is not updated when the config
list changes.

This ADR does not choose which definition should win.

Status remains Open.
