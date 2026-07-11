# mac client - notes for Claude

Swift menu-bar app, SwiftPM, ZERO third-party dependencies (hard
constraint; everything needed ships with macOS). Approved design:
`../docs/superpowers/specs/2026-07-10-are-you-up-design.md`.

## Commands

- `make build` / `make run` / `make test` / `make install` /
  `make uninstall`
- `ARE_YOU_UP_HOME=<dir>` redirects all state (config, db, log) for
  tests and E2E runs; `ARE_YOU_UP_DEBUG=1` logs every sample tick.

## Architecture

- `Sources/AreYouUpCore/` - all logic, fully tested: Timestamps, Store,
  Config, Log, Syncer, IdleTime, WallClockIdle.
- `Sources/AreYouUp/` - deliberately untested AppKit glue: main,
  AppDelegate (timers, wiring), StatusItemController (menu),
  HistoryStripView (6h strip). Keep it thin; new logic goes in Core
  where XCTest can reach it.

## Invariants (do not break)

- `Store` is main-thread-only. `Syncer` marshals every URLSession
  callback to the main queue before touching it; keep that discipline.
- `Syncer.syncOnce` is NOT re-entrant; `AppDelegate` guards ticks with
  `syncInFlight`. Do not remove the guard.
- Never mark samples synced without verifying the server's
  `{"accepted": N}` ack equals the batch size. A bare 200 is not an ack
  (captive portals answer 200 to anything), and `pruneSynced` makes a
  false ack permanent data loss.
- `pruneSynced` deletes only synced rows; unsynced rows are data the
  server has not seen.
- `idle_s` on the wire is wall-clock. `IdleTime`'s raw stopwatch pauses
  during sleep (dark wakes would report seconds instead of hours);
  always feed it through `WallClockIdle` (ADR-0008).
- Timestamps go through `Timestamps` (RFC 3339, local offset). Sqlite
  TEXT ordering is only approximately chronological across offset
  changes: fine for the documented housekeeping/display uses, unsound
  for anything correctness-critical.
