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
