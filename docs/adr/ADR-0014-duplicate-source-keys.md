# ADR-0014: Duplicate source keys in the Waukesha parcel layer

Status: Accepted
Date: 17 October 2026

## Context

Load 53 on 17 October 2026 failed Assertion B. The loader assembled
163,461 rows with 163,458 distinct (county_fips, muni_code, taxkey)
values.

The county layer Web_Tax_Parcel/FeatureServer/2 returns two features, with
different OBJECTIDs, for each of three taxkeys: VNT 2156998004,
VNT 2156998005 and VNT 2156998006 (municipality VNV, units 1 to 3 at
S110W23450 County Line Rd). The service lists 165,314 distinct OBJECTIDs
and returns the same count, so the duplicates are in the county data and
are not a paging artifact.

Inside each pair every field the loader maps is identical: muni_code,
owner_name, site_address, both mail lines, the three values, prop_class
and rec_source. The fields that differ are OBJECTID, GlobalID, GISAcres,
NotTrueArea, Shape__Area and Shape__Length, and the loader maps none of
them. One feature in each pair has NotTrueArea = 1 and the same 7.078 acre
polygon on all three units. Across the whole layer 15,730 of 165,314
features (9.5 percent) carry NotTrueArea = 1. The field name suggests the
polygon area is not the true area. The county definition has not been
read.

The three taxkeys have no row on any earlier load. Load 48 holds 163,394
rows and 163,394 distinct taxkeys, and no taxkey in load 48 or in
current_parcels appears under more than one muni_code. The config comment
on collapse_duplicate_geometry said Waukesha taxkeys were unique and that
collapse stays off. The county data no longer supports that comment.

## Decision

Set collapse_duplicate_geometry = true in config/dr_data_icp/entity.toml.

When fetched features share a taxkey and every mapped field matches, keep
the feature with the lowest OBJECTID and count the others in
records_skipped. When any mapped field differs, fail the load and name the
taxkey and the field. Fields the loader does not map are not compared.
Assertion B stays on the composite key and is not changed. The rule
applies to config/dr_data_icp only. config/milwaukee_parcels is not
changed.

The behaviour already exists in collapse_duplicate_geometry in
crates/radar-store/src/parcels.rs and is covered by three tests. No code
changes.

## Consequences

If the county data is unchanged, the next load should write 163,458 rows
and skip 1,856. This is arithmetic, not a promise. Assertion A checks it.

The survivor in all three pairs is the feature with NotTrueArea = 1. No
mapped field is affected. Any later mapping of acreage must treat polygon
area as unknown where NotTrueArea = 1, and must not read acreage from a
collapsed survivor. The acreage ADR decides this.

Dropped OBJECTIDs are printed for the first 50 taxkeys and are not stored.
The load row keeps one combined skipped total. Per-reason skip counts and
dropped identifiers cannot be recovered from the database. Until the load
accounting ADR fixes that, the load report must keep the console lines
that start with duplicate_geometry.

Collapse groups by taxkey alone and Assertion B uses county, municipality
and taxkey. No taxkey currently sits under two municipalities. If one
does, the load fails on a municipality difference. That failure is
intended.

## Rejected alternatives

Rejected: add OBJECTID to the composite key. Rejected because each unit
would be stored twice and every count would double, and OBJECTID is the
county's internal row id and can change when the layer is republished.

Rejected: relax or remove Assertion B. Rejected because it is the check
that kept the duplicates out of the database.

Rejected: keep any one row without comparing mapped fields. Rejected
because two records that disagree would be merged without anyone seeing
it.

Rejected: wait for the county to remove the duplicates. Rejected because
the timing is outside our control and the stored data would stay on the
23 August vintage.

## Not decided here

Writing NULL instead of 0 in records_skipped on a failed load (the column
allows NULL). Per-reason skip counts. Requesting a stable order and
asserting distinct OBJECTIDs against the service count. Two diagnostic
printers that choose the latest load by MAX(id) without a status filter.
Acreage fields.
