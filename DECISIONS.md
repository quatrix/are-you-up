# Architectural decision records

## 0001 - Input-only activity detection

**Date:** 2026-07-10 | **Status:** accepted
**Context:** The motivating scenario (watching video without touching the
keyboard) is invisible to input-idle detection. Media playback is
detectable via IOPM display-sleep assertions (verified, see LAB_NOTES.md
2026-07-10).
**Decision:** Track only mouse/keyboard input. User's explicit choice for
the MVP; it keeps the client to a single trivial signal.
**Alternatives:** (a) Count media playback as activity via IOPM
assertions - rejected: adds a second signal plus process-name filtering
(caffeinate holds the same assertion type and would read as "watching
video all night"). (b) A third `media` state - rejected with (a).
**Consequences:** Simplest possible client. Long passive-watching
stretches will read as idle; if that turns out to matter against whoop
data, revisit via a new decision (the raw-sample schema can grow a column
without breaking anything).

## 0002 - Clients ship raw idle seconds; states derive at query time

**Date:** 2026-07-10 | **Status:** accepted
**Context:** The active/idle threshold will need tuning against whoop
data, and the right value is unknown up front.
**Decision:** Clients report `(ts, idle_s)` samples every 30s and never
classify. `GET /v1/intervals` takes `threshold_s` as a query parameter.
**Alternatives:** Client-side classification with a baked-in threshold -
rejected: wrong threshold choices would permanently corrupt historical
data and require client redeploys to fix going forward.
**Consequences:** Threshold is tunable forever, historical data is
re-interpretable. Costs ~2900 rows/day/device of storage (negligible) and
puts derivation logic on the server (where it is unit-testable anyway).

## 0003 - Swift for the mac client

**Date:** 2026-07-10 | **Status:** accepted
**Context:** The client needs an NSStatusItem menu with a custom history
view, CoreGraphics idle queries, sqlite, and HTTP.
**Decision:** SwiftPM executable, zero third-party dependencies; all
needed frameworks ship with macOS.
**Alternatives:** Rust (matches the server and user tooling preferences) -
rejected: menu bar UI would go through objc2/tray-icon crates, adding
dependencies and friction exactly where the platform SDK is free.
**Consequences:** Two languages in the repo; in exchange the client stays
dependency-free and idiomatic on the platform.

## 0004 - RFC 3339 timestamps with local offset everywhere

**Date:** 2026-07-10 | **Status:** accepted
**Context:** Timestamps cross two databases, one API, and later analysis
against whoop data.
**Decision:** All timestamps are RFC 3339 strings carrying the device's
local UTC offset, stored as TEXT verbatim. Comparisons parse to instants.
**Alternatives:** Unix seconds UTC - rejected by user: wants proper
timestamps with timezone. UTC strings + separate offset column - rejected:
two fields where one self-documenting string works.
**Consequences:** Data is human-readable and preserves local time-of-day.
Range queries must parse rather than compare strings (mixed offsets do not
sort lexicographically); at this data volume that is irrelevant.

## 0005 - Pin the backend's Rust toolchain via `rust-toolchain.toml`

**Date:** 2026-07-10 | **Status:** accepted
**Context:** `cargo test` failed to compile `rusqlite`'s `bundled` sqlite
build script under the dev machine's default toolchains: the default
`nightly` (1.94.0, 2025-12-15) and `stable` (1.92.0) both predate
stabilization of the `cfg_select!` macro that `libsqlite3-sys 0.38.1`
uses (see LAB_NOTES.md 2026-07-10). A `1.96.1` toolchain happened to
already be installed locally and builds cleanly.
**Decision:** Add `backend/rust-toolchain.toml` pinning `channel =
"1.96.1"`. `rustup` auto-selects it for any `cargo` invocation under
`backend/`, independent of whatever channel is set as the ambient
default on a given machine or CI runner.
**Alternatives:** (a) Downgrade `rusqlite`/`libsqlite3-sys` to a version
predating the `cfg_select!` usage - rejected: trades a forward-looking
fix for a backward pin on a dependency we want to stay current, and does
not prevent the same class of breakage from recurring on the next
`cargo update`. (b) Do nothing and rely on developers having a
sufficiently new default toolchain - rejected: not reproducible, exactly
the failure mode observed here. (c) `RUSTC_BOOTSTRAP=1` to unlock
unstable features - rejected: does not apply (the crate lacks the
`#![feature(cfg_select)]` opt-in, so bootstrap mode changes nothing) and
would be fragile even if it did.
**Consequences:** Builds are reproducible regardless of ambient
toolchain state. Cost: a specific patch version is now pinned in the
repo and must be bumped manually as the project's minimum-supported-Rust
needs change; that is standard, low-effort toolchain maintenance.
