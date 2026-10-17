# ADR-0011: Two-stage Accela occupancy retrieval

Status: Accepted
Date: 17 September 2026
Decision owner: Zubia Mughal

## Context

Milwaukee occupancy certificates are the strongest current new-business
signal found this week. They live on Accela Citizen Access at
`aca-prod.accela.com/MILWAUKEE`, which is also LMS. Occupancy is Building
record type Occupancy (`NOCC-…`). Login is not required to search.

Task One (17 September 2026) confirmed this is not a CKAN CSV and not a
simple GET. Search is an ASP.NET POST with `__VIEWSTATE` and session
cookies. There is no `__EVENTVALIDATION` on this tenant. A 30-day
Occupancy window returned 167 records. `robots.txt` on this host is HTTP
404, which is not LIRA's `User-agent: * / Disallow: /`.

The in-page CSV export (`Export2CSV.ashx` after an AJAX postback in the
same session) contains only the results-grid columns. The fields that
identify a newly opening business exist only on `Cap/CapDetail.aspx`,
reached by `capID1`, `capID2`, and `capID3` from search-result links.
Those cap IDs are not in the CSV. A GET of a known CapDetail URL works
without a prior search.

No date of birth, driver's license, or SSN field exists on the occupancy
detail form. The applicant block does include a street address that is
not the occupancy site (Humboldt Blvd and 95th Street versus the
business parcels) and, on `NOCC-26-01549`, a field labeled Home Phone.

## Decision

### Two-stage pull, not a single export

Stage one: session GET of Building CapHome, POST Occupancy search for a
date window, paginate the HTML, harvest Record Number plus cap IDs from
result links. Non-Occupancy types in that window are excluded and
counted, not ingested.

Stage two: unauthenticated GET of each CapDetail URL. Rate-limit those
GETs. A failed detail fetch still stores the grid-stage row with rich
fields null and `detail_retry` set. It is not dropped.

CSV export is not the retrieval path for rich fields. It cannot supply
cap IDs. Using it as the only pull would permanently omit Legal Business
Name, DBA, Owner, Reason for applying, and the applicant contact that is
allowed to be stored.

### Confirmed field distribution

Grid and CSV (list stage, `value_source=csv_grid` when that is all that
landed): Date, Record Number, Record Type, Project Name, Status,
Address, Short Notes.

Detail page only (`value_source=detail_page` when the GET succeeds):
applicant name, Work Phone when so labeled, Owner and owner mailing
address, Legal Business Name, DBA, Reason for applying, business type,
hours, food, alcohol, parking, prior use, vacancy duration, and the
other Application Information labels on that form.

### Privacy boundary (this source, not inferred from LIRA)

Out of schema, named from live records:

- Applicant street address. On `NOCC-26-01548` the applicant line was
  2221 N. Humboldt Blvd. while the occupancy site was not that street.
  On `NOCC-26-01547` the applicant line was 5605 N. 95th Street. These
  read as home addresses.
- Any field labeled **Home Phone**. `NOCC-26-01549` used that exact
  label.

Stored when labeled **Work Phone** (`NOCC-26-01548`, `NOCC-26-01547`):
business-level contact, same treatment as LIRA's Licensee, Agt pattern.

If a future phone has no label, or a label other than Work Phone or Home
Phone, store it as `unlabeled_phone` and set a manual-review flag. Do
not guess.

Date of birth, driver's license, and SSN are absent on this form and
are not columns.

### Repeatable poll

Start Date is the day after the newest stored occupancy Date. End Date
is empty. Task One confirmed 15 September 2026 with End Date empty
returned the same 19 records as 15–17 September. Natural key is Record
Number. A rerun does not update rows.

### Join

Occupancy site Address joins to `current_parcels.site_address` with the
same alphanumeric collapse already used for owner names, after cutting
the city/state/zip tail. Match rate is reported. Unmatched rows stay.

## Rejected alternatives

Rejected: CSV-only ingest. Rejected because the export lacks cap IDs and
every rich field.

Rejected: guessing Home Phone versus Work Phone from the number. Rejected
because Task One saw both labels on this form.

Rejected: storing applicant street because LIRA stores a licensee
address. Rejected because on this form that street is not the site.

Rejected: requiring a successful detail GET before insert. Rejected
because a transient Accela failure would silently drop a new occupancy.
