# are-you-up: MVP design

**Date:** 2026-07-10 | **Status:** approved

## Purpose

Track keyboard/mouse activity on personal devices (macbook now, pixel later)
and expose it as active/idle time intervals over a REST API. The data is a
correction signal for whoop's time-in-bed detection, which misreads sofa
laptop sessions as bed time.

## Key decisions

1. **Input-only detection.** Only mouse/keyboard input counts as activity.
   Media playback is deliberately not tracked (a working detection via IOPM
   display-sleep assertions was prototyped and rejected: user choice, keeps
   the client minimal).
2. **Raw samples, query-time derivation.** The client reports
   `(ts, idle_s)` pairs; it never decides active/idle. The threshold is a
   query parameter on the server, so it stays tunable against whoop data
   without redeploying clients or rewriting stored data.
3. **Swift client, Rust server.** The client is a SwiftPM executable with
   zero third-party dependencies (AppKit, CoreGraphics, sqlite3, URLSession
   all ship with macOS). The server is axum + rusqlite + chrono.
4. **Timestamps are RFC 3339 strings with local offset** (e.g.
   `2026-07-10T23:41:03+03:00`) in the API, both databases, and logs.
   Instants stay unambiguous across DST/travel, and the data self-documents
   local time of day.
5. **`source` is a free-form device name** (`"macbook"`, `"pixel"`) sent
   with every upload and stored per sample.

## Architecture

```
mac/       Swift menu-bar app (SwiftPM executable)
backend/   Rust REST server (axum + rusqlite)
android/   (later)
```

Data flow:

1. Client samples seconds-since-last-input every **30s** via
   `CGEventSource.secondsSinceLastEventType(.combinedSessionState, anyInput)`
   (verified on target machine 2026-07-10: no TCC permission needed;
   `anyInput` is `CGEventType(rawValue: ~0)`, i.e. `kCGAnyInputEventType`).
   Each sample `(ts, idle_s)` is appended to local sqlite with `synced=0`.
2. Every **60s** the client POSTs up to 1000 unsynced rows per batch to
   `POST /v1/samples` and marks them synced on 200. The server upserts on
   `(source, ts)`, so retries are idempotent.
3. `GET /v1/intervals` derives active/idle intervals from samples at query
   time.

Communication runs over tailscale; the server is not internet-exposed and
the MVP has no auth.

## Mac client (`mac/`)

AppKit accessory app (no dock icon), components:

- **Sampler** - 30s timer, appends `(ts, idle_s)` to the store (`idle_s`
  rounded to a whole non-negative integer). Pause stops the timer; paused
  time is simply a gap in the data.
- **Store** - sqlite at `~/Library/Application Support/are-you-up/client.db`
  (WAL mode). Single `samples(ts TEXT PRIMARY KEY, idle_s INTEGER, synced
  INTEGER)` table. Synced rows older than 7 days are pruned.
- **Syncer** - 60s timer, POSTs unsynced rows via URLSession in batches of
  up to 1000, marks them synced on 200, records last-success time. Failure:
  log, retry next tick (no backoff; the scale does not need it).
- **Menu bar** - `NSStatusItem`. Icon: filled circle = active, dimmed =
  idle (display-only threshold 120s), pause glyph = paused. Menu: current
  status, "Last sync: Nm ago", a 6-hour history strip (custom view drawing
  colored segments from the local db), Pause/Resume, Quit.
- **Logger** - plain text to `~/Library/Logs/are-you-up.log`,
  tail-friendly: sync results and errors at info, sampling ticks at debug.
- **Config** - `~/Library/Application Support/are-you-up/config.json`,
  `{"server_url": ..., "source": "macbook"}`, written with defaults on
  first run.
- **Autostart** - LaunchAgent plist plus a `make install` that copies the
  binary and loads the agent. No .app bundle in the MVP.

## API

```
POST /v1/samples
{"source": "macbook",
 "samples": [{"ts": "2026-07-10T23:41:03+03:00", "idle_s": 4}, ...]}
-> 200 {"accepted": 120}
```

400 with a reason on malformed payload (unparseable ts, negative idle_s,
empty source). Upsert on `(source, ts)`.

```
GET /v1/intervals?from=...&to=...&threshold_s=120&source=macbook
-> 200 {"intervals": [
     {"source": "macbook",
      "start": "2026-07-10T22:00:12+03:00",
      "end":   "2026-07-10T23:15:42+03:00",
      "state": "active"}, ...]}
```

`from`/`to` required (RFC 3339); a sample is in range when
`from <= ts < to`. `threshold_s` optional, default 120. `source` optional;
default is all sources, intervals computed per source.

Derivation: a sample is *active* if `idle_s < threshold_s`, else *idle*.
Consecutive same-state samples from the same source merge while the gap
between them is <= 90s (3x the sample period); larger gaps break the
interval. An interval's `start`/`end` are the timestamps of the first and
last sample in the merged run (no extrapolation past observed samples).
Time not covered by samples is absent from the response; the consumer
treats it as no-signal.

```
GET /healthz -> 200 "ok"
```

## Server (`backend/`)

axum + rusqlite behind a `Mutex<Connection>` (about 3 writes/minute; a pool
would be decoration). Schema:
`samples(source TEXT, ts TEXT, idle_s INTEGER, PRIMARY KEY(source, ts))`,
WAL mode. Config via env: `ARE_YOU_UP_ADDR` (default `127.0.0.1:8080`;
deployment sets the tailnet address) and `ARE_YOU_UP_DB` (default
`./are-you-up.db`).

## Error handling

- Server unreachable: samples accumulate locally, syncer retries every 60s
  forever. Pruning only touches synced rows, so nothing is lost.
- Mac asleep / lid closed: timers do not fire; the gap correctly reads as
  no-signal.
- Screen locked: no special handling; locked means no input, so `idle_s`
  grows on its own.
- Client crash/reboot: unsynced rows persist in sqlite; sync resumes on
  next start.
- Server never 500s on bad input, only on genuine internal failures.

## Testing

- **Server:** unit tests for interval derivation (threshold
  classification, coalescing, 90s gap break, multi-source separation,
  empty range). Integration: POST -> GET roundtrip on a temp db,
  idempotent double-POST, validation rejections.
- **Client:** XCTest for the store (insert, mark-synced, prune, 6h window
  query) and the syncer against a stubbed URLProtocol (batching,
  marks-on-200, keeps-on-failure). The AppKit shell stays thin and
  untested.
- **E2E smoke:** script starts the real server on a random port, POSTs a
  synthetic day of samples, asserts the derived intervals.

## Out of scope (MVP)

- Android client (design accommodates it via `source`)
- Auth/TLS (tailscale is the perimeter)
- Media-playback detection
- .app bundle, code signing, notarization
- Dashboards/visualization beyond the 6h menu strip
