# Lead Radar

Local-first ICP search and lead intelligence system for Dr. Data Decision Intelligence LLC. Rust, Tauri desktop shell, SQLite. Reads public records, parcels, business license filings, occupancy filings, patent assignments, liquor licenses, and produces ranked, sourced prospect lists per client vertical.

No job title lives in this file. This is documentation, not a hiring document.

## Start here

Read in this order before touching anything:

1. `docs/adr/ADR-0001` — why config is loaded at runtime, not compiled in
2. `docs/adr/ADR-0008` — why current parcel state is materialized, not derived live on every query
3. `docs/adr/ADR-0010` — the privacy exclusion pattern, why it exists, and why it is non-negotiable
4. `crates/radar-store/src/parcels.rs`
5. `crates/radar-store/src/groups.rs`
6. the detection module

Then run the app once before changing anything.

## Architecture

Four real pieces:

- `crates/radar-config` — schema, validation, session state
- `crates/radar-store` — migrations, data access, detection, extraction, adapters
- `apps/radar-desktop` — Tauri shell, plain HTML/JS/CSS, no frontend framework
- `config/<vertical>/` — one directory per client. Adding a vertical is a new config directory, never a code change

## Non-negotiable design rules

**Insert-only, enforced at the database level.** A SQLite authorizer blocks UPDATE and DELETE on core tables, not just convention. The database runs in WAL mode with a busy_timeout set on every connection, app, loaders, and tests alike. This was a real bug: a rollback-journal connection with no timeout caused live lock contention between the running app and a concurrent adapter fetch.

**A missing value is never a guess.** Null, not-applicable, and unreadable are three different facts and must render differently everywhere. A blank phone field on an Accela occupancy record does not mean extraction failed. It can mean the only phone on the form was labeled Home and was correctly excluded.

**Config drives behavior.** Direction, weights, bands, vocabulary live in TOML. A hardcoded threshold in Rust is a bug being written.

**ADR before a non-trivial architecture change. Profile a source with real requests before writing an adapter for it.** Every source in this repo was profiled first, and every profile found something the documentation did not say: LIRA's CSV export uses different column names than the search-results grid. Accela's export contains only six grid columns, none of the rich fields, which live only on a per-record detail page reached by a session-scoped capID. Do not assume a field exists or a site behaves as documented.

**A materialized table exists only if it can be fully rebuilt from source-of-truth tables.** Never a second source of truth.

## The multi-vertical model

Composite key is `(county_fips, muni_code, taxkey)`, never taxkey alone. Waukesha and Milwaukee taxkeys can collide.

## Data sources currently wired

| Source | Provides | Retrieval | Status |
|---|---|---|---|
| Waukesha parcels | Ownership, assessed value | Adapter, live fetch | Live |
| Milwaukee parcels | Ownership, different field shape than Waukesha | Adapter, live fetch | Live |
| USPTO patent assignments | Business addresses, separated from filing attorney | Adapter | Live |
| Milwaukee liquor licenses | New, ended, changed licensees | Nightly snapshot diff | Live |
| LIRA license applications | New restaurant filings | Manual CSV export, then adapter reads the file | Live |
| Milwaukee Accela occupancy | New business establishment filings, with reason code and applicant | Two-stage session pull: search for capIDs, then a GET per detail page | Live |
| FRED economic series | Mortgage rates, housing starts, ten series | Adapter, API key required | Live |
| PDF vision extraction | Structured fields from scanned license applications | Manual URL list, then Claude vision call per document | Live, small batch validated |

## What is NOT automated, and why

`itmdapps.milwaukee.gov` disallows all agents except Googlebot in robots.txt. This governs both the LIRA CSV export and the scanned PDF attachment pulls. Both require a human to manually retrieve files into a folder; the loaders only read what is already there.

Accela's occupancy portal (`aca-prod.accela.com`) has no robots.txt at all. That source is retrieved by a session-based search plus a per-record GET, rate-limited, but not blocked by site policy the way LIRA is. The two hosts are treated differently for a reason. Do not assume the same restriction applies to both.

## Running locally

Required environment variables, never committed, never logged, read from the process environment only:

- `ANTHROPIC_API_KEY`
- `FRED_API_KEY`

Run the app:

