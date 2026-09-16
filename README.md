REPO: C:\Users\zumah\DrData_ICP_Lead_radar

Confirm working directory. Do not commit or push yet.

VERIFY FIRST, before writing anything

List every file in crates/radar-store/migrations/. Confirm whether
018 genuinely does not exist, or was renamed/removed. State plainly
whether it's a gap or an artifact.

List every file in docs/adr/. Print each filename and its title
line. Confirm whether ADR-0005 and ADR-0006 do in fact share the
same title, and if so, note this in the README as a known small
inconsistency rather than hiding it.

List every binary under crates/radar-store/src/bin/.

Run cargo test with an isolated CARGO_TARGET_DIR and report pass or
fail plainly. Do not guess if you have not actually run it this
session.

WRITE README.md AT REPO ROOT

Audience: an engineer new to this codebase. No job title anywhere
in the file, this is documentation, not a hiring document.

Sections, in this order:

1. One paragraph: what this is. Local-first ICP search and lead
   intelligence system for Dr. Data Decision Intelligence LLC. Rust,
   Tauri desktop shell, SQLite. Reads public records (parcels,
   business license filings, patent assignments, liquor licenses)
   and produces ranked, sourced prospect lists per client vertical.

2. "Start here." A suggested reading order for someone new:
   ADR-0001 first (why config is runtime, not compiled in), then
   ADR-0008 (why current state is materialized, not derived live),
   then parcels.rs, then groups.rs, then the detection module. Then
   run the app once before touching anything.

3. Architecture. The four real pieces: crates/radar-config (schema,
   validation, session state), crates/radar-store (migrations, data
   access, detection, extraction), apps/radar-desktop (Tauri shell,
   plain HTML/JS/CSS, no framework), config/<vertical>/ (one
   directory per client, TOML files, no code changes needed to add
   a vertical).

4. Non-negotiable design rules, stated plainly, each with why:
   - Insert-only, enforced at the database level by a SQLite
     authorizer, not just by convention.
   - A missing value is never a guess. Null, not-applicable, and
     unreadable are three different facts and must render
     differently everywhere.
   - Config drives behavior. Direction, weights, bands, vocabulary
     all live in TOML. A hardcoded threshold in Rust is a bug being
     written.
   - Write an ADR before a non-trivial architecture change. Profile
     a data source with real requests before writing an adapter for
     it, never assume a field exists or a site behaves as documented.
   - A materialized table exists only if it can be fully rebuilt
     from the source-of-truth tables. Never a second source of
     truth.

5. The multi-vertical model. Composite key is (county_fips,
   muni_code, taxkey), not taxkey alone. Explain briefly: Waukesha
   and Milwaukee taxkeys can collide, and adding a county or vertical
   is a new config directory, never new Rust.

6. Data sources currently wired. A short table: source, what it
   provides, how it's retrieved, automation status. Include parcels
   (Waukesha and Milwaukee, different field shapes per county),
   USPTO patent assignments, Milwaukee liquor licenses (nightly
   snapshot diffing), LIRA license applications, FRED economic
   series, PDF vision extraction.

7. What is NOT automated, and why. itmdapps.milwaukee.gov disallows
   all agents except Googlebot in robots.txt. This governs both the
   LIRA CSV export and the PDF attachment pulls. Both require a
   human to manually retrieve files into a folder; the loaders only
   read what's already there. State this plainly so nobody tries to
   build a crawler against that host later.

8. Running locally. Required environment variables
   (ANTHROPIC_API_KEY, FRED_API_KEY), never committed, never logged,
   read from the process environment only. How to run the app. How
   to run tests, and the actual current pass/fail state you just
   verified. Note that data/radar.db is gitignored, so a fresh clone
   starts with an empty database and every source needs to be
   fetched or loaded from scratch.

9. Binaries reference. A table of every binary you found in
   src/bin/, one line each on what it does.

10. Privacy boundary, stated as a hard rule, not a guideline. Vision
    extraction from scanned government documents never extracts
    date of birth, driver's license number, social security number,
    or any personal phone or home address, even when clearly visible
    on the form. Reference ADR-0010. This line does not move.

11. Known open items. List plainly: the migration 018 gap (as
    confirmed), the ADR-0005/0006 title duplication (if confirmed),
    that ranking currently uses two attributes, that a config for
    any vertical beyond the two currently checked in does not exist
    yet, and that exports/dr_data_icp_ranked.csv is a point-in-time
    snapshot, not something regenerated automatically.

No em dashes anywhere in the file.

Print the full README.md content after writing it. Do not commit.
Stop after writing and printing it so it can be reviewed.
