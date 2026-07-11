# are-you-up

Tracks activity on my devices (keyboard/mouse on the mac, screen
usage on the pixel) and serves it as
active/idle intervals, as a correction signal for whoop's time-in-bed
detection (sofa laptop sessions are not bed time).

## How it works

```
mac menu-bar app                 rust server                 whoop tooling
 samples idle_s every 30s  --->  stores raw samples    --->  GET /v1/intervals
 buffers in local sqlite   POST  (sqlite, verbatim)          active/idle ranges
 syncs batches every 60s         derives at query time
```

- The client reads seconds-since-last-input via `CGEventSource` (no
  permissions needed), buffers samples in local sqlite, and syncs in
  batches. Unsynced data survives server outages, crashes, and reboots;
  the server upserts on `(source, ts)` so retries are harmless.
- The server stores raw samples and classifies active/idle at query
  time: `threshold_s` (default 900, i.e. 15 minutes without input) is a
  query parameter, so the threshold stays tunable against whoop data
  forever without touching stored data or redeploying clients.
- Gaps in samples (lid closed, machine off, client paused) are simply
  absent from results: no signal, not "idle".
- Timestamps are RFC 3339 with the device's local UTC offset,
  everywhere. Transport is plain HTTP over a private tailnet; there is
  deliberately no auth.

## Quickstart

Server (any box on the tailnet):

    cd backend && make run          # cargo run -- --help for options (--addr, --db)

Client (this mac):

    cd mac && make install          # release build + LaunchAgent, starts at login
    # then point it at the server:
    #   ~/Library/Application Support/are-you-up/config.json -> "server_url"

Query (note: percent-encode `+` in timestamps as `%2B`):

    curl "http://<server>:8080/v1/intervals?from=2026-07-10T22:00:00%2B03:00&to=2026-07-11T08:00:00%2B03:00"

Full API reference: `backend/README.md`. Client paths, menu, and env
vars: `mac/README.md`. Android setup: `android/README.md`.

## Layout

- `backend/` - Rust REST server (axum + rusqlite)
- `mac/` - Swift menu-bar client (SwiftPM, zero third-party deps)
- `android/` - Kotlin phone client (no resident process; 15-min job replays system usage events)
- `docs/superpowers/specs/` - approved design; `docs/superpowers/plans/` - implementation plans
- `DECISIONS.md` - architecture decision records; `LAB_NOTES.md` - empirical findings; `SESSION.md` - known limitations and deferred work

## Development

Each part has a Makefile with `build`, `run`, and `test`:

    make -C backend test            # 19 tests: unit (derivation) + integration (API)
    make -C mac test                # 24 tests: store, syncer, config, log, timestamps
    make -C android test            # JVM unit tests: synthesizer, store, syncer, timestamps
    make -C backend smoke           # E2E: real server process, asserted intervals
    scripts/e2e.sh                  # joint E2E: real client syncing to real server
