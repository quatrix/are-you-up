# android client - notes for Claude

Kotlin app, single Gradle module, framework-only: NO third-party
runtime dependencies (hard constraint, mirrors `mac/`). Test-only
exceptions: Robolectric, `org.json:json` (the mockable android.jar
stubs org.json for JVM tests), and MockWebServer (android.jar omits
JDK-internal modules like com.sun.net.httpserver). Approved design:
`../docs/superpowers/specs/2026-07-11-android-client-design.md`; ADRs
0006 (screen-interactive signal) and 0007 (no resident process).

## Commands

- `make build` / `make test` / `make install` / `make run` /
  `make log` / `make clean`
- Force a job run: `adb shell cmd jobscheduler run -f dev.areyouup 1`
- The SDK path comes from `local.properties` (gitignored); see
  README.md prerequisites.

## Architecture

- `app/src/main/java/dev/areyouup/core/` - all logic, fully tested:
  `Timestamps`, `Synthesizer` (events -> samples; the one real
  algorithm), `Sample`/`SampleQueue`, `Store`, `Syncer`.
- `app/src/main/java/dev/areyouup/` - deliberately untested thin glue:
  `SampleJob` (JobService), `MainActivity`, `Prefs`. New logic goes in
  `core/` where JUnit can reach it.

## Invariants (do not break)

- ADR-0007: no resident process. Never add a foreground service,
  broadcast receiver, alarm, or wakelock; `SampleJob` is the only thing
  that ever runs in the background, and the JobInfo carries no
  constraints beyond the period (it must run offline so samples
  buffer).
- The cursor (in `Prefs`) only advances after synthesis AND insertion
  succeeded; advancing it early silently loses usage forever.
- Never mark samples synced without verifying `{"accepted": N}` equals
  the batch size. A bare 200 is not an ack (captive portals), and
  `pruneSynced` makes a false ack permanent data loss.
- `pruneSynced` deletes only synced rows.
- Timestamps go through `core/Timestamps` (RFC 3339, local offset,
  ADR-0004). `ts` TEXT ordering in sqlite is housekeeping-only.
- minSdk = 34: do not add legacy compatibility branches.
