# ADR-0012: WAL mode and busy_timeout on every store connection

Status: Accepted
Date: 18 September 2026
Decision owner: Zubia Mughal

## Context

A workspace test that opened `data/radar.db` directly failed with
SQLite `database is locked`. The connection was in rollback journal
mode and had no `busy_timeout`. In that default, a second connection
that wants the file while another holds it fails immediately. That is
not a rare case in this repo. The Tauri app holds a long-lived
connection. Loaders and live adapters write. Tests sometimes open the
same file. Any two of those at once can collide.

The failure was not a bad assertion and not a unique bug in one
binary. It was the journal mode and timeout that every opener
inherited from SQLite's defaults.

## Decision

Every connection opened anywhere in this codebase, the app, the
loaders, and the tests, is configured the same way:

WAL mode (`PRAGMA journal_mode = WAL`) on file-backed databases.
A 5 second busy timeout (`SQLITE_BUSY_TIMEOUT`) on every
connection, including in-memory ones.

`configure_connection` in `crates/radar-store/src/migrate.rs` is the
single place that does this. `migrate` and `apply_migrations` call
it. Openers that do not migrate still call it (for example
`import_karl` on a read-only source, and tests that open the live
store). In-memory databases skip WAL because SQLite has no file to
put a wal sidecar next to; they still get the timeout.

## Rejected alternatives

Rejected: catching `database is locked` in the one test that failed
and retrying. That hides the same collision in the app and in
loaders.

Rejected: serialising all access through one process. The usage
shape is concurrent: one long-lived app reader, occasional adapter
writes, occasional test reads. WAL is what SQLite provides for that
shape.

Rejected: a longer busy timeout as the only change, leaving rollback
journal mode. Readers would still block on writers instead of
proceeding, and a writer would still wait on any reader. WAL is what
lets a reader proceed while a writer is active.

## Consequences

WAL creates two sidecar files next to the database,
`radar.db-wal` and `radar.db-shm`. The main `.db` file alone is not
guaranteed complete if it is copied outside the app without those
sidecars. A copy of `data/radar.db` by itself can be a truncated
view of what the process last wrote.

`.gitignore` already ignores `data/*.db-*`, which matches both
sidecar names. They are not committed. They are still required next
to the live file while the process is using WAL.

Read-only connections that cannot change journal mode (for example
a URI open of another process's file) skip the WAL pragma and keep
the timeout.
