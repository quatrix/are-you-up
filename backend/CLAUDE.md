# backend - notes for Claude

Rust REST server (axum + rusqlite). Approved design:
`../docs/superpowers/specs/2026-07-10-are-you-up-design.md`. ADRs:
`../DECISIONS.md` (0002 raw-samples/query-time thresholds, 0004
timestamps, 0005 toolchain pin).

## Commands

- `make build` / `make run` / `make test` / `make smoke` (E2E against a
  real server process)
- Logging: `tracing` with `RUST_LOG` (default `info`). All non-2xx
  responses log centrally in `error_response` (5xx at error, 4xx at
  debug); per-request traces come from tower-http's `TraceLayer`. Route
  new log lines through those conventions, not println/eprintln.
- The toolchain is pinned by `rust-toolchain.toml` (1.96.1): older
  toolchains cannot compile `libsqlite3-sys 0.38`'s build script
  (ADR-0005). Do not remove the pin without checking that.

## Architecture (three files, keep it that way)

- `src/main.rs` - thin binary: clap CLI (`--addr`, `--db`; each falls
  back to `ARE_YOU_UP_ADDR`/`ARE_YOU_UP_DB` then a default - keep that
  precedence, scripts rely on the env vars), bind, serve.
- `src/lib.rs` - open_db, router, handlers, validation. Handlers
  hand-parse bodies/queries so every client mistake returns a uniform
  JSON 400 (axum's extractors would 422/plain-text some of them).
- `src/intervals.rs` - pure derivation logic, the only real logic here;
  fully unit-tested, no I/O.

## Invariants (do not break)

- Client input must never produce a 500; validation runs before any db
  work.
- No handler panics while holding the db Mutex (the lock `expect`s
  state this invariant), and never `.await` while holding it.
- POST batches are all-or-nothing: one transaction, rollback-on-drop,
  pinned by `post_samples_rolls_back_whole_batch_on_db_error`.
- Timestamps are stored as RFC 3339 TEXT verbatim. NEVER range-filter
  or sort by `ts` in SQL: mixed UTC offsets make TEXT ordering unsound.
  Parse to instants, then compare (this is why the full scan exists).
- `threshold_s` stays a query parameter (ADR-0002); never bake
  classification into storage.

## Known ceilings (deliberate, documented)

- `GET /v1/intervals` full-scans and parses every row: measured ~0.8s
  and ~100-150MB transient RSS at 1M rows (LAB_NOTES.md 2026-07-10).
  Upgrade path: epoch column + index. Do not "optimize" past this
  without reading that entry.
- `consolidate=true` adds a sweep that is O(n^2) in the count of active
  intervals: measured 233ms at 8k actives, sub-ms at day scale
  (LAB_NOTES.md 2026-07-11). Irrelevant until someone consolidates
  year-wide ranges on top of the full scan above.
- No schema migrations (`CREATE TABLE IF NOT EXISTS` only); see
  `../SESSION.md`.
