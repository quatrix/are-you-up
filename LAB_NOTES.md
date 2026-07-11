# Lab notes

## 2026-07-10 - Probed macOS idle-detection APIs for the mac client

Verified on this macbook (macOS 16.0, Swift 6.1.2):

- `ioreg -c IOHIDSystem -d 4 | grep HIDIdleTime` returned
  `"HIDIdleTime" = 66589625` (nanoseconds, ~0.07s while actively typing).
  Works without permissions, but requires shelling out or IOKit registry
  walking.
- `CGEventSource.secondsSinceLastEventType(.combinedSessionState,
  eventType: CGEventType(rawValue: ~0))` from Swift (`tmp/probe.swift`)
  returned `31.43` after ~30s without input. No TCC/accessibility prompt.
  `CGEventType(rawValue: ~UInt32(0))` (aka `kCGAnyInputEventType`) is
  accepted by the Swift initializer. Chosen as the sampling API: in-process,
  no parsing, no permissions.
- `CGSessionCopyCurrentDictionary()` exposes `CGSSessionScreenIsLocked`
  (absent when unlocked). Readable, but the design needs no lock handling:
  locked implies no input.
- `pmset -g assertions` showed Arc holding `NoDisplaySleepAssertion named:
  "Video Wake Lock"` while playing media, i.e. video playback IS detectable
  via IOPM assertions. Caveat found: running `caffeinate` holds the same
  assertion type, so a naive check misreads caffeinate as media. Rejected
  for MVP anyway (user chose input-only detection).

## 2026-07-10 - Probed Timestamps formatter edge cases during mac Task 1 review

Ran a standalone probe (`scratchpad/ts_probe.swift`, mirror of
`Timestamps`) plus a clean `swift build -Xswiftc
-strict-concurrency=complete` in the mac-client worktree:

- The `DateFormatter` with `"yyyy-MM-dd'T'HH:mm:ssXXXXX"` + `en_US_POSIX`
  is DST-aware: a winter instant formats as `+02:00` and a summer instant
  as `+03:00` under Asia/Jerusalem, so offsets are computed per-date, not
  snapshotted.
- Ruled out the "cached formatter snapshots the timezone" hypothesis: after
  `NSTimeZone.default = Asia/Tokyo`, the same static formatter instance
  emitted `+09:00`. The formatter consults the default zone dynamically.
  (A *system* timezone change while the agent runs still depends on
  Foundation resetting its systemTimeZone cache; unverified in-process.)
- Parse strictness: `date(from:)` returns nil for legal RFC 3339 variants
  we never emit - fractional seconds (`...T22:00:00.123+03:00`) and
  lowercase `t`/`z`. Fine while it only parses its own output; a doc
  comment should state the accepted subset.
- Clean build with `-strict-concurrency=complete`: zero warnings. Recent
  SDKs mark `DateFormatter` Sendable (thread-safe since macOS 10.9), so the
  shared `static let formatter` is safe even off-main.

## 2026-07-10 - `rusqlite --features bundled` fails to build on the default toolchain

`cargo test` in `backend/` failed before any of our code ran:

```
error[E0658]: use of unstable library feature `cfg_select`
  --> libsqlite3-sys-0.38.1/build.rs:110:9
```

`rustc --version` showed the machine's default `nightly` toolchain resolves
to `1.94.0-nightly (2025-12-15)`. `libsqlite3-sys 0.38.1`'s bundled-build
script calls the `cfg_select!` macro unconditionally (no `#[cfg(...)]` or
`#![feature(cfg_select)]` guard), assuming it is stabilized; it is not
stable as of that nightly.

Checked `rustup toolchain list`: besides the default `nightly` (1.94) and
`stable` (1.92.0, also predates the fix), a `1.96.1` toolchain
(2026-06-26) was already installed locally. `rustup run 1.96.1 cargo build`
succeeded, confirming `cfg_select!` is stable by 1.96.1 and the failure is
purely a stale-default-toolchain problem, not a bad dependency pin.
Decision recorded in DECISIONS.md 0005.

## 2026-07-10 - GET /v1/intervals full-table-scan cost at scale (Task 4 quality review)

Measured during the Task 4 quality review, to put a real number behind
`get_intervals`'s "full scan + parse" approach rather than the earlier vague
"if a profile ever shows this mattering" comment: 1M rows (one device-year
of 30s samples) seeded into the `samples` table via a recursive-CTE insert,
then queried with a realistic `from`/`to` range against a running server.

Result: ~0.8s per request once warm, and server RSS climbed 22MB -> 115MB
-> 170MB across two consecutive requests (first request pays for loading +
parsing all rows into memory; the second still grows further, consistent
with the `Vec<(String, String, i64)>` intermediate plus the derived
`intervals::Sample` vectors both being retained per-request until the
response is built and dropped).

Conclusion: at this scale, memory pressure on a small host (e.g. a
Raspberry Pi or a small VPS) is the binding constraint before request
latency would be - 170MB transient RSS for one query is a meaningful
fraction of a small host's total RAM. Revisit with an epoch column + index
(to push range filtering into SQL instead of scanning + parsing every row
in the table) if a deployment target's memory budget is tight enough for
this to matter before a redesign is otherwise warranted.

## 2026-07-11 - Android Task 1 scaffold: wrapper provenance and test-config probes (quality review)

Three empirical checks run against the committed gradle scaffold
(commit 2d0c2f8), each in a scratchpad copy so the shared worktree
stayed untouched.

**Wrapper provenance.** The committed `gradle-wrapper.properties`
contains `retries=0` and `retryBackOffMs=500`, which Gradle 8.9 does not
generate. Cause: the system Gradle is 9.6.1, and `gradle wrapper
--gradle-version 8.9` (plan Task 1 Step 4) emits the *generating*
version's wrapper files with only `distributionUrl` pointing at 8.9.
Verified integrity by generating a reference wrapper in a scratch
project with the same system Gradle: committed jar sha256
`497c8c2a7e50...` is byte-identical to pristine 9.6.1 output, and both
`gradlew` scripts diff clean. So the wrapper is authentic, just
9.6.1-flavored; the extra keys are 9.6.1 generation defaults. Expect
churn (keys dropped, jar replaced) if `./gradlew wrapper` is ever run,
since 8.9 will regenerate its own files.

**Robolectric without `isIncludeAndroidResources`.** The scaffold
declares Robolectric but no `testOptions` block. Probed whether Task 5's
planned StoreTest pattern (`RuntimeEnvironment.getApplication()` +
framework sqlite, in-memory db, `@Config(sdk = [34])`) needs the
standard `unitTests.isIncludeAndroidResources = true`: a probe test with
exactly that shape passed without it. No android *resources* are
touched, so the option genuinely isn't needed; adding it would be cargo
cult. (Robolectric's first run downloaded android-all as the plan
predicts.)

**`android.useAndroidX=true` comment accuracy.** Flipped the flag to
`false` and re-ran tests: build fails in
`:app:compileDebugUnitTestKotlin` with AGP's "Configuration
`:app:debugUnitTestRuntimeClasspath` contains AndroidX dependencies, but
the `android.useAndroidX` property is not enabled", listing Robolectric's
androidx.test transitive deps - exactly what the gradle.properties
comment claims. The flag is load-bearing for tests despite the
zero-androidx runtime.

## 2026-07-11 - Pixel 7 usage-event probe: spec assumptions confirmed

Ran the Task 2 probe (dump button -> logcat) on the physical Pixel 7
(Android 16, API 36) after granting Usage Access, following a scripted
lock/peek/unlock sequence. 82 screen/keyguard events in the last 2h,
epoch-ms timestamps, strictly ordered, matching wall clock (latest event
11:56:53+03:00 vs 11:57:04 logcat line).

Confirmed:
- All four event types appear as assumed: SCREEN_INTERACTIVE /
  SCREEN_NON_INTERACTIVE / KEYGUARD_HIDDEN / KEYGUARD_SHOWN.
- Unlock ordering is SCREEN_INTERACTIVE first, KEYGUARD_HIDDEN 1.5-7s
  later (lock-screen dwell + face unlock). Windows therefore correctly
  start at KEYGUARD_HIDDEN, excluding lock-screen dwell.
- Lock-screen peeks (screen on without unlocking - ambient/notification
  checks, typically exactly ~10s) produce SCREEN pairs with no
  KEYGUARD_HIDDEN and are common (dozens in 2h). The screenOn&&unlocked
  state machine correctly ignores them.
- Events from BEFORE the app was installed are returned (2h window
  fully populated minutes after install): retention is system-side, so
  job downtime loses nothing within retention.

Deviations from the spec's mental model (both benign):
- On this device KEYGUARD_SHOWN re-engages 0.7-5s after every
  SCREEN_NON_INTERACTIVE, so the "re-light inside the lock delay with
  no keyguard events" scenario the Synthesizer supports is rare here;
  the common no-keyguard-event pattern is the lock-screen peek, which
  the same state machine handles.
- DEVICE_SHUTDOWN/STARTUP not observed (no reboot during the window);
  handling remains defensive-only until a reboot happens to be covered
  by a later dump.

Conclusion: the retrospective event-replay design (ADR-0007) is sound
on the target device; Synthesizer event mapping needs no changes.

## 2026-07-11 - java.time `ofPattern` digit-locale probe (android Task 3 quality review)

Question: `Timestamps.kt` builds its formatter with
`DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ssXXX")` and no
explicit `Locale`, while the mac twin deliberately pins
`en_US_POSIX`. Would a device set to a non-Latin-digit locale (e.g.
ar-EG, Eastern Arabic numerals) emit digits chrono can't parse?

Ran a single-file JDK 21 repro (`java -Duser.language=ar
-Duser.country=EG LocaleCheck.java`) formatting
1_783_764_000_000 ms in Asia/Jerusalem:

    formatted=2026-07-11T13:00:00+03:00
    decimalStyle=DecimalStyle[0+-.]
    localizedBy=٢٠٢٦-٠٧-١١T١٣:٠٠:٠٠+03:00

Conclusion: `ofPattern` hard-codes `DecimalStyle.STANDARD` (ASCII
digits) regardless of the default locale; only an explicit
`.localizedBy(...)` switches digit sets. The missing `Locale` in
`Timestamps.kt` is therefore NOT a bug - java.time differs from
`SimpleDateFormat`/Darwin `DateFormatter` here, which is why the mac
twin needs the pin and the android one doesn't. Also verified both
test epoch constants map to their claimed UTC instants
(1783764000000 = 2026-07-11T10:00:00Z, 1768471200000 =
2026-01-15T10:00:00Z) via Python `datetime`.

## 2026-07-11 - Synthesizer degenerate-input probes (android Task 4 quality review)

Probed `Synthesizer.synthesize` with inputs outside its documented
contract via a temporary JUnit file (removed after the run), plus
verified every cross-reference its comments make. Findings:

- **Clock skew (`nowMs` < `cursor.tsMs`), interactive cursor**: emits
  exactly one sample at the past `nowMs` and returns a cursor regressed
  to `nowMs` (probe: `samples=[-60000]` for now = cursor - 60s). The
  regression is self-healing: events are level-based (each sets a bit
  absolutely), so the next run's re-replay of the overlap re-derives
  identical windows and `INSERT OR IGNORE` dedupes the re-emitted rows.
  The lone spurious sample is only wrong if the clock jumped back past
  the open window's start. Non-interactive cursor: no samples, cursor
  regresses, harmless. Noted in SESSION.md for Task 7 to clamp.
- **Event exactly at `nowMs`**: processed (filter is `> nowMs`), closes
  the window at `nowMs` with correct state bits carried into the cursor
  (`screenOn=true, unlocked=false` after a LOCKED-at-now). Correct per
  the `(cursor, now]` contract, though no committed test pins the
  inclusive end.
- **Unsorted input** (contract violation - close event listed before
  opens): `emitGrid(start > end)`'s `while (t < endMs)` never runs, so
  it emits the single end sample and terminates - bounded degradation,
  no exception, no runaway allocation.
- **Rerun at the same `now`** (open window): second run emits exactly
  `[now]`, the same millisecond as the first run's final sample, so the
  ts strings are identical and local/server dedupe absorbs it -
  idempotent, confirming the header comment's boundary-duplicate claim.
  Second-granularity ts collisions are generally harmless here: every
  sample this source emits is `idle_s=0`, so colliding rows are always
  identical.
- **Comment cross-references all verified**: 90s merge gap =
  `backend/src/intervals.rs:6` (`MAX_GAP_S: i64 = 90`), server upsert =
  `backend/src/lib.rs:138` (`ON CONFLICT (source, ts) DO UPDATE`),
  local `INSERT OR IGNORE` matches the spec's Store section (forward
  reference; Task 5 not yet built).
- **Scale math**: a 10h open window emits 1201 boxed longs (~30KB); a
  pathological 7-day fully-interactive backlog ~20k (~500KB transient).
  Memory is a non-issue at the 15-min job cadence.
- The `windowStart = -1L` sentinel collides with valid negative
  timestamps in principle (an event at -1 ms opening a window would be
  treated as "no window"; one closing it would grid from -1), but is
  unreachable: the cursor starts at install-time `now` and events <=
  cursor are skipped, so no non-modern timestamp ever passes the filter.

## 2026-07-11 - Store WAL-in-init probe under Robolectric (android Task 5 quality review)

Verified the two claims in `Store.kt`'s init comment ("No effect on
in-memory databases (tests); WAL on the device") with a temporary
Robolectric test (`WalProbeTest.kt`, deleted after the run) that opened
a file-backed and an in-memory `Store` and queried `PRAGMA
journal_mode` plus `isWriteAheadLoggingEnabled`:

- File-backed (`name = "probe.db"`): `journal_mode=wal`,
  `isWriteAheadLoggingEnabled=true`. So calling
  `setWriteAheadLoggingEnabled(true)` in `init` - i.e. before the first
  `writableDatabase` access - does stick: the helper records the flag
  and applies it as an open flag in `getDatabaseLocked`. Robolectric
  shadows only the native layer and runs the real AOSP
  `SQLiteOpenHelper` code over real sqlite, so this exercises the same
  framework path the device uses.
- In-memory (`name = null`): `journal_mode=memory`, flag reported
  false, inserts work, no crash - the WAL request is silently ignored,
  as the comment claims.

Also noted while reading: `SQLiteDatabase` keeps a per-connection LRU
cache of compiled statements keyed by SQL text (default 25), so
`markSynced`'s per-row `execSQL` inside one transaction reuses the
compiled UPDATE rather than re-parsing 1000 times - the mac twin's
explicit prepared-statement reuse happens implicitly here. At ~3
batches/day this was never a hot path anyway.

## 2026-07-11 - com.sun.net.httpserver is uncompilable in AGP unit tests

The android plan's Task 6 test harness (a real local
com.sun.net.httpserver.HttpServer, JDK stdlib) fails to compile in the
app module's test source set: AGP compiles unit-test Kotlin/Java against
the compileSdk android.jar stub (here platforms/android-34/android.jar),
and android.jar only declares the java.*/com.sun.* subset that exists in
ART's libcore - jdk.httpserver was never Android API. The host JDK the
tests later RUN on is irrelevant; symbol resolution happens against the
stub jar.

Ruled out (by direct experiment, all reverted): a jvmTarget/--release
cross-compilation restriction (javac --release 17/21 compiles the import
fine on this JDK), forcing test-compile targets to the host JDK version
(fails identically), -Xadd-modules=jdk.httpserver (no effect). Only the
android.jar-as-compile-classpath explanation fits.

Consequence: Syncer tests use MockWebServer 4.12.0 (test-only dep) as
the real loopback socket server; spec decision 4 and the plan were
amended. General lesson for this repo: JVM-unit-test code in the android
module may only import android.jar-visible APIs, regardless of runner.

## 2026-07-11 - Syncer adversarial-response probes (android Task 6 quality review)

Three temporary MockWebServer probes (`ReviewProbeTest.kt`, deleted
after the run) against the real `Syncer`, plus two code traces:

- **Ack as JSON string**: a server answering `{"accepted": "1"}`
  (string, not number) passes the ack check - org.json's `getInt`
  coerces numeric strings. Lenient, but fail-safe: coercion can only
  admit a *correct* count in string form; a wrong count or non-numeric
  string still fails. Left as-is.
- **Trailing slash in serverUrl**: `Syncer("http://host:port/", ...)`
  produces request path `//v1/samples` (observed via the dispatcher),
  which a path-routed server 404s -> `Failed(status 404)`, rows stay
  unsynced. Fail-safe but a config footgun; the mac twin is immune
  (`appendingPathComponent` normalizes). The plan's Task 7 UI trims
  `trimEnd('/')` on input, which papers over it; noted in SESSION.md.
- **Non-ASCII source**: `source = "pixel-ünïcode-עברית"` round-trips
  byte-exact (Kotlin `toByteArray()`/`decodeToString()` default UTF-8,
  matching the backend's UTF-8-only axum String extractor).
- **Duplicate-ts livelock trace (ruled out twice over)**: the feared
  case - a batch with two equal ts strings making `accepted !=
  batch.size` and livelocking the drain - is unreachable. (1) Batches
  come from `Store.nextBatch`, a SELECT over `ts TEXT PRIMARY KEY`, so
  a batch cannot contain duplicate ts strings; the Synthesizer *can*
  emit two instants formatting to the same second (window end + next
  window start <1s apart), but `INSERT OR IGNORE` collapses them before
  any batch exists. (2) Even with duplicates, `post_samples` returns
  `accepted = req.samples.len()` (request array length, backend
  lib.rs:148), not rows-changed, so the ack would still match.
- **Unread errorStream on non-200 is not a leak**: the early return
  skips `errorStream`, which only forfeits keep-alive reuse - and
  `disconnect()` in `finally` forfeits that anyway by closing the
  socket. At ~3 batches/day, connection reuse is worthless here.

## 2026-07-11 - SampleJob threading and schedule-guard analysis (android Task 7 quality review)

Reasoned from SDK contracts (no device attached in this review; the
on-device confirmations are listed for Task 9). Verified the build:
`./gradlew test assembleDebug` green, 33 tests x2 variants.

- **Same-job overlap**: JobScheduler serializes executions of one job
  id - a periodic job is "active" from onStartJob until jobFinished or
  onStopJob completes, and the next period will not launch while it is
  active. The only overlap window is `onStopJob -> true` while the
  untracked worker thread keeps running (nothing interrupts it), after
  which a rescheduled run can start a second `runOnce` concurrently.
  Traced the consequences: SharedPreferences is a per-file in-process
  singleton with internally synchronized reads/writes, so racing cursor
  writes interleave to one of two near-identical values; the overlapped
  span is re-replayed idempotently (level-based events, local `INSERT
  OR IGNORE`, server upsert on `(source, ts)`). Two Store instances on
  one WAL db can at worst throw `SQLiteDatabaseLockedException` on
  concurrent write, which the job's catch-all turns into
  log-and-retry-next-run. Concluded: safe by idempotence, no fix
  needed - but the safety chain is non-obvious and deserves a comment
  in SampleJob.
- **jobFinished after onStopJob**: the framework ignores completion
  callbacks for a job it no longer considers active (logs a warning,
  no exception). No double-jobFinished path exists: one thread per
  onStartJob, one jobFinished per thread.
- **schedule() guard vs JobInfo changes**: persisted JobScheduler jobs
  survive `adb install -r` (package update does not cancel jobs; only
  force-stop, data clear, or uninstall do). The `getPendingJob(JOB_ID)
  != null` early-return therefore pins the OLD JobInfo forever across
  the project's own documented upgrade flow (`git pull && make
  install`): a future PERIOD_MS change would silently never apply.
  Comparing `pending.intervalMillis != PERIOD_MS` (the getter returns
  the clamped value, so this is stable for periods >= the 15-min
  platform floor) keeps phase preservation in the common case and
  self-heals on upgrades.
- **Prefs.apply() ordering**: apply() mutates the in-memory map
  synchronously and queues the disk write; job-thread writes are
  visible to the UI thread immediately (same singleton). The only loss
  window is process death before the queued write commits - the cursor
  regresses at most one run and the replay is idempotent, so no
  read-after-write bug exists in-process.

## 2026-07-11 - Android on-device E2E: full pipeline + recovery paths verified on the Pixel 7

Ran the Task 9 smoke plus the T7 review's on-device checklist against
the real backend (100.88.181.84:8080). Results:

- Job arming and first run: opening the activity logged "job scheduled:
  every 15 min, persisted" and JobScheduler fired an immediate first
  run: "0 events, 0 samples, synced 0" - correct (cursor starts at
  install time, no backfill).
- Real usage: after ~2 min of scripted phone use, a forced run produced
  "8 events, 6 samples, synced 6" and the server derived an exact
  active interval (13:39:38-13:41:18+03:00, source=pixel). Background
  job process CAN read usage events and POST cleartext over the tailnet
  (checklist b, g).
- Activity open during a run: clean, no SQLiteDatabaseLockedException (f).
- Force-stop parks the persisted job ("Could not find job 1"); reopening
  re-arms it and state returns to "waiting" (e).
- Forced mid-run timeout: unreachable in practice - the run completes in
  well under a second, cmd jobscheduler timeout found "No matching
  executing jobs". The onStopJob overlap window is theoretical at this
  runtime; idempotence covers the remainder (d).
- Reboot: job state "waiting" after boot WITHOUT opening the app; the
  post-boot run captured 5 events including the shutdown and synthesized
  the post-unlock samples (c).
- Unplanned bonus - live server-unreachable recovery: tailscale had not
  reconnected after boot, so two consecutive runs failed with connect
  timeouts ("sync FAILED after 0: request failed: failed to connect...")
  while samples kept buffering. Once tailscale was up, one run flushed
  everything: "6 events, 11 samples, synced 14" - 4 buffered + 11 new -
  1 boundary duplicate collapsed by INSERT OR IGNORE (the arithmetic
  confirms live dedupe). The server then showed two active intervals
  with the reboot correctly absent as a no-signal gap.
- adb install -r over the running install preserves the armed job
  ("waiting" immediately after reinstall) - the documented upgrade flow
  is safe (c, second half).

Not yet observed (will confirm passively over the coming days): item
(h), overnight Doze deferral followed by a catch-up run. Nothing in the
design depends on it beyond what (c)-(g) already proved.

Conclusion: the retrospective event-replay client works end to end on
the target device, including every recovery path we could trigger.

## 2026-07-11 - Live deployment E2E: client backlog flush + interval derivation verified

Pointed the installed mac client at the production backend
(100.88.181.84:8080, tailnet) by editing `server_url` in
`~/Library/Application Support/are-you-up/config.json` and restarting the
LaunchAgent (`launchctl kickstart -k gui/$UID/com.are-you-up.mac`).

Observed in `~/Library/Logs/are-you-up.log`: hours of `sync failed after 0
samples, will retry: server returned status 405` (the port-8080 collision
noted in SESSION.md - an unrelated local service answered 405; the ack
check correctly refused to mark anything synced), then immediately after
restart `synced 244 samples` - the entire buffered backlog flushed in one
batch.

Verified server-side with
`GET /v1/intervals?from=2026-07-11T08:00:00%2B03:00&to=...&source=macbook`:
returned 6 intervals matching real usage - a no-data gap before 08:11
(laptop asleep, >90s sample gap yields no interval), a merged 1.5h active
block (~180 samples), a zero-length idle interval at 09:49:21 (single
sample where idle_s crossed the 900s threshold before input resumed 30s
later - expected artifact of per-sample threshold classification), and
timestamps echoed verbatim as RFC 3339 `+03:00`.

Conclusion: the offline-buffer -> ack-verified sync -> query-time
derivation pipeline works end to end against a real deployment, and the
captive-portal-style failure mode (bare non-ack responses) demonstrably
loses no data.
