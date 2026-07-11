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

## 0006 - Android activity signal is screen-interactive, not input-only

**Date:** 2026-07-11 | **Status:** accepted
**Context:** The android client needs an activity signal. Android has no
public global seconds-since-last-input API, so the mac's ADR-0001
semantics cannot be replicated cheaply.
**Decision:** The pixel counts as active while the screen is on and the
keyguard is dismissed; everything else is a no-signal gap. Samples carry
`idle_s=0` during interactive windows, so the server derives only
`active` intervals or gaps for this source. The screen timeout itself is
an input-driven idle detector, which keeps this close to input-only in
practice.
**Alternatives:** `UsageStatsManager` `USER_INTERACTION` events would
give true seconds-since-last-touch (hands-off video would read idle),
but need event-behavior probing, more code, and buy little at a 15-min
threshold; an AccessibilityService observing input is invasive overkill.
**Consequences:** Media watching with the screen on reads as active,
diverging from ADR-0001 on the mac. If whoop comparisons show this
matters, the event source can be swapped without touching the API
contract. No permissions beyond Usage Access (required anyway by 0007).

## 0007 - Android client has no resident process; a 15-min job reads the system usage log

**Date:** 2026-07-11 | **Status:** accepted
**Context:** Hard requirement: imperceptible battery/CPU/memory footprint
and no visible presence (no persistent notification).
**Decision:** No foreground service, receivers, timers, or alarms. A
persisted `JobScheduler` periodic job (15 min) replays
`UsageStatsManager` screen/keyguard events from a stored cursor,
synthesizes 30s-grid samples retrospectively, syncs, and exits. The OS
records the events whether or not the app runs; the job's process lives
seconds per cycle.
**Alternatives:** A foreground service observing SCREEN_ON/OFF broadcasts
live (the mac-like design) works without Usage Access but pins a
resident process and a permanent notification - exactly what the
requirement forbids. WorkManager adds an AndroidX dependency for nothing
JobScheduler lacks here.
**Consequences:** Zero background footprint and free crash/reboot
recovery (events are system-side; reruns catch up). Costs: the one-time
Usage Access grant, up to ~15-30 min upload latency (harmless: queries
target past days, timestamps come from the event log), force-stop parks
the job until the app is reopened, and correctness depends on
`queryEvents` behavior - verified by an on-device probe as the
implementation plan's first task.

## 0008 - Mac idle is wall-clock, derived from the stopwatch via uptime delta

**Date:** 2026-07-11 | **Status:** accepted
**Context:** `CGEventSource.secondsSinceLastEventType` counts awake time
and pauses during sleep. A closed lid dark-waking hourly runs a couple
of 30s sample ticks per wake, so reported idle_s crept +30s/hour and
never crossed the 900s threshold: the server showed hourly "active"
blips while nobody was home (LAB_NOTES 2026-07-11).
**Decision:** `WallClockIdle` converts the stopwatch to wall-clock
seconds since last input. `ProcessInfo.systemUptime` pauses during
sleep identically, so with no input the stopwatch grows by exactly the
uptime delta between ticks; a reading below that growth means an input
event reset it, and `now - raw` pins the event to wall clock. Report
`now - lastInputDate`.
**Alternatives:** Suppressing the first sample after a >90s tick gap -
rejected: multi-tick wakes still blip (observed 19:56:23 + 19:56:53
pair). `NSWorkspace` sleep/wake notifications - rejected: not reliably
delivered during dark wake, and untestable in Core; the uptime-delta
rule needs no OS callbacks and unit-tests with injected tuples.
**Consequences:** Samples taken during dark wakes now honestly report
hours of idle, and awake-but-untouched behavior is unchanged (uptime and
wall clock advance together, so the value passes through). Costs: idle
state lives in the app (a stopwatch reset between process restarts is
re-anchored on first tick), and a stray input event during a dark wake
(e.g. a Bluetooth peripheral in a bag) still counts as activity - which
is arguably correct.
