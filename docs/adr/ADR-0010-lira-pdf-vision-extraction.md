# ADR-0010: Vision extraction of Milwaukee license application PDFs

Status: Accepted
Date: 15 September 2026
Decision owner: Zubia Mughal

## Context

Milwaukee LIRA application packets are public records. CSV export
already lands in `lira_applications`. The attached application PDF
is a separate document, reached only by `ApplAttachServlet?id=...`
links a human copies from Application Search. Preflight found no
PDF download path, no Anthropic client, and no ADR-0006 enrichment
table.

One packet (Belli's Bistro LLC, 13 pages) was inspected by hand.
`pdffonts` reported no embedded fonts on any page. There is no
selectable text layer. The packet is a scan. The same packet
contains at least three form layouts: a Business Information Form,
a Business License Plan of Operation with eleven numbered sections,
and further supplemental pages. Sound amplification sits in Plan of
Operation section 3e. Security cameras, count, and locations sit in
5d to 5e. Building owner name, phone, and address sit in 9g to 9h.

itmdapps.milwaukee.gov `robots.txt` allows Googlebot and disallows
every other user-agent on `/`. This tool must not crawl, enumerate,
or request that host on its own.

A prior request asked for personal applicant fields taken from the
same class of scanned government form, including home phone. That
request was declined because the values identify a private
individual. This schema is built so that request cannot be satisfied
by accident.

## Decision

### Vision, not OCR, not text parsing

Extraction uses Claude vision on page images. Text parsing is
rejected because there is no text layer to parse. Pure OCR is
rejected because the fields that matter include handwritten values
and checkboxes whose meaning depends on nearby printed labels, and
because OCR would still require a second layout-aware step this
scan does not support. Vision reads label and value together on the
page image.

No new Cargo crate is added for the model client. The call uses the
existing `ureq` HTTP client. The API key is read from
`ANTHROPIC_API_KEY` in the process environment. It is never written
to a file, never logged, and never placed in the prompt. The prompt
contains the page images and the extraction schema only.

Page images are produced by a small local rasterizer that uses
Python libraries already present on this machine (`pypdfium2` and
Pillow). That is not an Anthropic SDK and not a crawl.

### Schema and label driven, not coordinates

Extraction is keyed on field names and the printed labels that
identify them. Coordinate templates are rejected because layout is
not identical across pages of one packet, and is not expected to be
identical across license types.

### Retrieval is a human-curated URL list

The downloader reads
`C:\Users\zumah\Desktop\radar_sources\milwaukee_lira_pdfs\urls.txt`.
Each line is one URL a human already opened and pasted. The tool
fetches only those URLs, with a normal browser User-Agent, at most
two attempts, and it skips a file already on disk. It does not
discover ids, walk Application Search, or follow links found in
HTML. A crawler is rejected because `robots.txt` disallows it and
because enumeration would be a crawl even with a browser UA.

### Personal identifiers are out of schema

The schema does not include date of birth, driver license number,
social security number, home phone, or any other personal identifier
of the applicant or agent, even where those values are visible on
the scan. `building_owner_phone` is the landlord or building-owner
contact for the premises, a property-level fact, and is the only
phone besides `business_phone` that may be stored. Owner or agent
personal phone is skipped even if the form asks for it. This records
the same ground on which the earlier personal-data request was
declined.

### Storage shape: own insert-only tables, not ADR-0006

ADR-0006 stores one enrichment field per row, keyed on a computed
owner `group_key`, for gradual enrichment of a business already on
the parcel roll. This extraction produces one full record per
application PDF, keyed on an application identifier taken from the
attachment URL (or from the saved filename when that is all that
exists). That identifier is not a column on `lira_applications`,
`entities`, or any enrichment table, because the enrichment table
does not exist.

Forcing this packet into ADR-0006 would invent a `group_key` from a
trade name, which is an identity join this step is forbidden to
add, and it would split one application into field rows that look
like website enrichment of a known owner group. They are not that.

This work uses its own insert-only pair:

- `lira_pdf_extractions`: one row per extraction of one PDF, holding
  the business-level fields, model identifier, timestamp, and
  source filename.
- `lira_pdf_extraction_fields`: one row per schema field on that
  extraction, holding the value, 1-based page number, confidence,
  and whether the field was unreadable.

The field table matches ADR-0006's grain (one field, with
provenance) without borrowing ADR-0006's identity (`group_key`) or
pretending the enrichment migration has been written. Re-extraction
inserts new rows. Nothing is updated in place.

`email_domain` and `email_is_free_provider` are derived in process
from `contact_email` when that address is readable. They are not
guessed by the model. Free providers are gmail.com, yahoo.com,
outlook.com, hotmail.com, and aol.com.

## Rejected alternatives

Rejected: parsing the PDF text layer. Rejected because `pdffonts`
showed none.

Rejected: a pure OCR pass (Tesseract or similar) as the extractor.
Rejected because checkboxes and handwriting still need label
context, and because it invites coordinate cuts of a layout that
already varies inside one packet.

Rejected: coordinate templates per form page. Rejected because one
application already contains at least three layouts.

Rejected: a crawler or any automatic request of Application Search
or attachment ids. Rejected because `robots.txt` disallows non-Google
agents on that host, and because the sanctioned path is a human
paste of specific attachment URLs.

Rejected: storing this extraction in the ADR-0006 enrichment table
shape keyed on `group_key`. Rejected because the identity is an
application packet, not an owner group, and because that table has
not been created.

Rejected: adding an Anthropic SDK crate. Rejected while `ureq`
already performs HTTPS JSON calls in this crate. Ask again before
adding a crate.

Rejected: putting home phone or other applicant personal identifiers
in the schema under any name. Rejected because those values identify
a private person and were already declined on a prior request.

## Constraints

The existing address join, liquor detectors, and config layer are
not changed by this ADR.

An unreadable field is stored null and named in `fields_unreadable`.
It is never a guess and never a default. An ambiguous checkbox is
`unknown`, not `no`. An absent form section for that license type is
`unknown`, not `no`. An unreadable camera count is null, not zero.

The first run is a demonstration on a small hand-pasted URL list.
It is not production volume.
