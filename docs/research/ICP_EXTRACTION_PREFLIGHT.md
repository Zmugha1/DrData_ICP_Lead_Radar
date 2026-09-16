# License application PDF extraction: preflight

Date: 15 September 2026
Working directory: `C:\Users\zumah\DrData_ICP_Lead_radar` (confirmed before any other work)
Status: investigation only. No extraction schema, ADR, or pipeline was written. Stopped pending architecture confirmation.

This file is the research report. The later 20-application validation comparison, if and when it is run, belongs in `docs/research/ICP_EXTRACTION_REPORT.md`.

## 1. How the license application PDF is retrieved, and where files land

PDFs are not being retrieved. There is no downloader, no `ApplAttachServlet` client, and no PDF on disk for this source.

What exists today is CSV-only:

- A human exports Application Search from `itmdapps.milwaukee.gov` with the page Export to CSV control.
- Files are dropped in `C:\Users\zumah\Desktop\radar_sources\milwaukee_lira\`.
- The only file there is `App_By_LicenseType.csv` (SSRS headers: `PROFESSION_FULL_NAME`, `LICENSEE`, `LICENSEE_ADDR`, `TYPE`, `APPLICATION_DATE`). No PDF, no attachment id.
- `load_lira_applications` loads that folder into `lira_applications`. The binary prints that it does not fetch `itmdapps`. The loader comment and migration 019 both say it never opens PDFs or `ApplAttachServlet`.

The earlier LIRA profile (`C:\Users\zumah\Desktop\radar_reports\milwaukee-lira-profile.md`) recorded that Show Details HTML contains a filename link to `ApplAttachServlet?id=…`. That link was not followed then, and nothing in this repo follows it now.

`lira_applications.has_attachment` is in the schema for the HTML-table Attachments Yes/No column. The loaded SSRS export has no that column, so all 3,712 rows have `has_attachment = NULL` (load_run 25 and 26, 1,856 rows each).

No application PDFs were found under the repo, `radar_sources`, Desktop (aside from unrelated files), or Downloads.

## 2. One application PDF: text layer, handwriting, layout

Cannot be answered from a file. There is no application PDF in this workspace or in the known source folder, so none was opened.

`ApplAttachServlet` was not fetched. Prior work on this source treated that as out of scope, and the extraction task asked to stop if files are not already retrieved.

Until a real application PDF is on disk, these architecture facts are unknown:

- selectable text versus flat scan
- machine-printed versus handwritten fields
- whether layout is identical across license types

Do not assume it is a scan. Do not assume a text layer. A hybrid design that parses text and sends only handwritten or checkbox regions to vision cannot be chosen until at least one, and preferably several license types, are inspected.

The HTML Show Details block is not a substitute for the PDF. That expansion has trade name, districts, premise description, phone, hours, and an attachment filename. It does not have email, a legal-entity field separate from business name, or owner/agent sections. Those live in the PDF if they exist at all.

## 3. ADR-0006 enrichment table: schema, row count, writers

ADR-0006 records this shape. It is design only. No migration created the table. `data/radar.db` has no table or view whose name contains `enrich`. Row count is 0 because the table does not exist.

Specified unit of storage: one field per row, insert-only, not one blob per company.

| Column / concept | What ADR-0006 says |
|---|---|
| entity identity | computed `group_key` (not a column on `entities` today) |
| field name | the enriched field |
| value | the extracted or seeded value |
| evidence quote | required on every read field |
| source_url | provenance |
| fetched_at | provenance |
| origin | enumerated; at least `manual` plus the adapter name |
| unknown | stored explicitly; absent row is not unknown |
| current view | `fetched_at` descending, then insert order; origin does not confer precedence |
| render_state | five states on the entity, default `not_yet_enriched` |

Nothing writes enrichment today. Inserts in this codebase go to load runs, entities, parcels, liquor, LIRA CSVs, detections, outcomes, series, and assignments. `docs/BUILD_SEQUENCE.md` Phase C ("The enrichment table per ADR-0003") is still unchecked.

ADR-0006 also already permits cloud extraction for public records at the call site, with client data never in the prompt. It does not implement that adapter.

Fit note for later, not built now: the proposed PDF extraction keys on `application_id` and a full application record. ADR-0006 keys on `group_key` and one field per row. Those are different grains. LIRA also has no `application_id` column today.

## 4. Anthropic client

None. No Anthropic crate, no `ANTHROPIC` env usage, no Claude call site.

The only HTTP client in `radar-store` is `ureq`, used for FRED (`FRED_API_KEY` from the process environment) and other fetches. No `.env` in the repo. No Python Anthropic client either.

## Stop

Architecture is not confirmed. Confirm after at least one application PDF is on disk, or after fetching `ApplAttachServlet` into a local folder is authorized. Until then, vision versus parse cannot be decided from evidence.
