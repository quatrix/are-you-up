# are-you-up: android client design

**Date:** 2026-07-11 | **Status:** approved

## Purpose

Report pixel phone usage to the existing are-you-up backend as a second
`source` ("pixel"), feeding the same whoop time-in-bed correction as the
mac client. Personal-use APK sideloaded on the owner's Pixel 7; never
distributed through the Play Store.

The API contract, timestamp rules (RFC 3339 with local offset, ADR-0004),
`source` semantics, offline buffering, and ack-verified sync all carry
over unchanged from the approved MVP design
(`2026-07-10-are-you-up-design.md`). This spec covers only what is new on
android.

## Hard requirement: imperceptible footprint

The app must be as close to zero cost as android allows: no measurable
battery, CPU, or memory footprint, and no visible presence (no persistent
notification). The owner should never "feel" it running. This requirement
drives the architecture below.

## Key decisions

1. **Screen-interactive is the activity signal.** The phone counts as
   active while the screen is on AND the keyguard is dismissed; all other
   time is a no-signal gap (like the mac asleep). This deliberately
   diverges from mac ADR-0001 (input-only): hands-off video watching with
   the screen on reads as active on the phone. Accepted because the
   screen timeout is itself an input-driven idle detector, and true
   input-only detection on android would need heavier machinery for
   marginal gain. Upgrade path if whoop data ever shows this matters:
   swap the event source; the contract does not change.
2. **No resident process: retrospective reading of the system's usage
   log.** Android's `UsageStatsManager` already records screen-interactive
   and keyguard events continuously, system-side, whether or not our app
   runs. A `JobScheduler` periodic job (15 min, `setPersisted(true)`)
   wakes, replays events since a persisted cursor, synthesizes samples,
   syncs, and exits. No foreground service, no notification, no broadcast
   receivers, no timers, no wakelocks, no alarms. Process lifetime is a
   couple of seconds per 15 minutes. Cost: the one-time Usage Access
   grant in Settings (acceptable for a sideloaded personal app) and up to
   ~15-30 min upload latency (irrelevant: whoop correction queries
   yesterday, and sample timestamps come from the event log, not the
   upload time).
3. **Synthetic 30s-grid samples keep the contract unchanged.** For each
   interactive window the job emits `(ts, idle_s=0)` samples: one at the
   window start, one every 30s after it, and one at the window end. The
   server's existing derivation (threshold 900, 90s gap merge) turns
   these into active intervals with no server changes. Consequence: the
   server only ever derives `active` intervals or gaps for `pixel`,
   never `idle` - correct under decision 1.
4. **Framework-only Kotlin.** Single Gradle module, no third-party
   runtime dependencies - the android twin of the mac's zero-dep
   constraint. sqlite via `android.database.sqlite`, HTTP via
   `HttpURLConnection`, JSON via `org.json` (ships in the framework),
   scheduling via `JobScheduler`, time via `java.time`. Test-only
   exceptions (never shipped in the APK): Robolectric, so `Store` tests
   run on the JVM instead of needing the phone plugged in; `org.json`,
   because the mockable android.jar stubs it for JVM tests; and
   MockWebServer, because android.jar omits JDK-internal modules like
   `com.sun.net.httpserver`, so a real loopback test server needs a
   library. `minSdk 34` (the Pixel 7 is past Android 14; no legacy
   branches).

## Architecture

```
android/
  Makefile                 build / test / install / log (gradlew + adb)
  README.md  CLAUDE.md     per-part convention, like backend/ and mac/
  app/src/main/java/dev/areyouup/
    core/                  pure logic, fully tested (mirrors AreYouUpCore)
      Timestamps.kt        RFC 3339 with local offset via java.time
      Synthesizer.kt       usage events -> interactive windows -> samples
      Store.kt             sqlite buffer (SQLiteOpenHelper, WAL)
      Syncer.kt            batch POST + ack verification
    SampleJob.kt           JobService: cursor -> events -> synthesize ->
                           store -> sync -> reschedule; thin glue
    MainActivity.kt        status + config screen; thin glue
  app/src/test/            JVM tests (JUnit + Robolectric for Store)
```

`core/` contains no android UI or service types and holds all logic;
`SampleJob` and `MainActivity` stay as thin as the mac's AppKit shell.
(`Store` uses android's sqlite classes, hence Robolectric; the other core
files are pure Kotlin/JDK.)

## Event processing (the one new algorithm)

The job persists a **cursor** in `SharedPreferences`: the instant up to
which samples have been synthesized, plus whether the screen was
interactive at that instant. Each run:

1. Query `UsageStatsManager.queryEvents(cursor_ts, now)`.
2. Fold events into interactive-state transitions. Interactive begins at
   `KEYGUARD_HIDDEN` (or `SCREEN_INTERACTIVE` when the keyguard is
   already dismissed, covering re-lighting the screen inside the lock
   delay); it ends at `KEYGUARD_SHOWN`, `SCREEN_NON_INTERACTIVE`, or
   `DEVICE_SHUTDOWN` (which closes any window left open by a power-off).
3. Emit the 30s-grid samples (decision 3) for every interactive span
   inside `(cursor_ts, now]`. A window still open at `now` (job fired
   mid-session) emits samples up to `now`; the cursor state carries
   "interactive" so the next run continues the same window seamlessly.
4. Insert with `INSERT OR IGNORE` (`ts` is the primary key), advance the
   cursor to `now`, then sync.

Duplicates are harmless end to end: local inserts ignore, the server
upserts on `(source, ts)`. First run ever starts the cursor at `now`
(no backfill; history before install is unknowable anyway).

**Design assumption to verify on-device before implementation** (like the
mac's CGEventSource probe): `SCREEN_INTERACTIVE` / `SCREEN_NON_INTERACTIVE`
/ `KEYGUARD_HIDDEN` / `KEYGUARD_SHOWN` events (API 28+) appear in
`queryEvents` with Usage Access granted, with sensible timestamps, and
are retained at least hours (the cadence only needs minutes; system
retention of usage events is days). The implementation plan's first task
is a logcat probe on the Pixel 7 confirming this.

## Store and sync

Identical rules to the mac client, so they are stated by reference:
schema `samples(ts TEXT PRIMARY KEY, idle_s INTEGER NOT NULL, synced
INTEGER NOT NULL DEFAULT 0)` in WAL mode; POST unsynced rows in batches
of up to 1000 (looping until drained, since a long-offline backlog can
exceed one batch); mark synced only when the ack `{"accepted": N}` equals
the batch size (a bare 200 is not an ack - captive-portal rule); prune
synced rows older than 7 days; failures keep rows and wait for the next
job run - no backoff, no retry state. `HttpURLConnection` with 30s
connect/read timeouts.

## Config and UI

`SharedPreferences` (the android-native config store, filling
config.json's role on the mac): `server_url` (no committed default -
the real endpoint is deployment config, typed into the app once on
first launch; blank means the job buffers without syncing), `source`
(default `"pixel"`), `paused` (default `false`).

`MainActivity`, one plain XML screen, is the only UI:

- Usage Access status, with a button opening the Settings grant page.
- Last job run and its result (windows found, samples synthesized,
  sync outcome), last successful sync time, unsynced row count.
- Pause toggle: while paused the job still runs and still syncs any
  buffered rows, but synthesizes nothing and advances the cursor - the
  paused span becomes a permanent gap, matching mac pause semantics.
- `server_url` text field with save.
- Opening the activity (re)schedules the job; first launch is what arms
  everything.

No notification of any kind. Logging goes to logcat, tag `are-you-up`,
one info line per job run; `adb logcat -s are-you-up` is the tail
(filling the mac's log-file role).

## Build, install, and operate

Toolchain: JDK 17+ and the Android SDK command-line tools (installable
via `brew install --cask android-commandlinetools` plus
`sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0"`);
no Android Studio required. The Gradle wrapper (`gradlew`) is checked in,
so builds need nothing else.

`android/Makefile`, following the backend/mac convention:

    make build      # ./gradlew assembleDebug -> app-debug.apk
    make test       # ./gradlew test (JVM unit tests, no device needed)
    make install    # build + adb install -r on the connected phone
    make run        # install + adb shell am start (launches the activity)
    make log        # adb logcat -s are-you-up (the tail)
    make clean      # ./gradlew clean

Getting it on the Pixel 7 (documented step-by-step in
`android/README.md`):

1. On the phone: Settings -> About phone -> tap Build number 7x to
   enable Developer options, then enable USB debugging.
2. Plug in via USB, accept the debugging prompt, `make install`.
3. Open the app once: grant Usage Access via the in-app button, check
   the server URL, done. The persisted job is armed from that point,
   surviving reboots; the phone never needs to be plugged in again.

Upgrading is `git pull && make install` (debug-keystore signatures
match, so `-r` reinstalls in place and the sqlite buffer, prefs, and
cursor survive). The APK is signed with the local debug keystore -
sufficient for sideloading on one owned device, and deliberately not a
release-signing setup.

## Error handling

- **Server unreachable:** rows buffer locally; next job run retries.
  Nothing is lost; pruning only touches synced rows.
- **Reboot:** `setPersisted(true)` re-arms the job automatically - no
  boot receiver. `DEVICE_SHUTDOWN` closes any open window; time powered
  off is a gap.
- **Force-stop:** android cancels persisted jobs until the app is next
  opened. Known ceiling, accepted for a personal device; reopening the
  activity reschedules, and the event log means the downtime's usage is
  still recovered (within event retention).
- **Doze:** periodic jobs defer while the device sits idle - exactly the
  periods with no new usage to report. On wake the job runs and catches
  up from the event log.
- **Clock/timezone changes:** timestamps are formatted per-instant with
  the zone rules in effect (java.time), so travel and DST behave per
  ADR-0004.
- **Malformed server response / partial ack:** rows stay unsynced, error
  logged, next run retries.

## Testing

- **Pure JVM JUnit:** `Timestamps` (format, offset, DST edges);
  `Synthesizer` (unlock/lock sequences, re-light inside lock delay,
  window open across job runs via cursor state, shutdown closing a
  window, sub-30s sessions, empty event ranges, paused runs);
  `Syncer` against a real loopback HTTP socket via MockWebServer
  (test-only dependency; the original plan's `com.sun.net.httpserver`
  cannot compile - AGP builds unit tests against android.jar, which
  omits JDK-internal modules, see LAB_NOTES.md 2026-07-11): batching,
  drain loop, marks-on-verified-ack, keeps-on-failure,
  keeps-on-partial-ack, keeps-on-non-JSON-200.
- **Robolectric:** `Store` (insert-or-ignore, unsynced query order,
  mark-synced, prune-only-synced).
- **Glue untested:** `SampleJob` and `MainActivity` stay thin, same rule
  as the mac shell.
- **E2E:** documented manual smoke in the README - install on the pixel,
  grant Usage Access, use the phone a minute, force a job run, then
  `GET /v1/intervals?source=pixel` against the real backend shows the
  session.

## Out of scope

- Play Store distribution, release signing (debug keystore only),
  ProGuard/R8 tuning beyond defaults.
- Input-only detection via `USER_INTERACTION` events (upgrade path noted
  in decision 1).
- Any notification, live/60s sync, history visualization on the phone.
- Backfilling usage from before the app was installed.
