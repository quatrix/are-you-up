# Android Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kotlin app for the Pixel 7 that reports screen-usage samples to
the existing backend as source `"pixel"`, with zero background footprint
(no resident process).

**Architecture:** A persisted 15-minute `JobScheduler` job replays the
system's screen/keyguard usage events from a stored cursor, synthesizes
30s-grid `(ts, idle_s=0)` samples for unlocked-screen windows, buffers
them in sqlite, and syncs with ack verification (ADR-0006, ADR-0007).
Pure logic lives in `core/` (fully tested); `SampleJob`/`MainActivity`/
`Prefs` are thin untested glue - the same split as `mac/`.

**Tech Stack:** Kotlin 2.0.21, AGP 8.7.3, Gradle 8.9, compileSdk/minSdk/
targetSdk 34, framework-only at runtime (`android.database.sqlite`,
`HttpURLConnection`, `org.json`, `JobScheduler`, `java.time`). Test-only:
JUnit 4.13.2, Robolectric 4.14.1, `org.json:json` (android.jar ships
stubbed org.json for JVM unit tests), MockWebServer 4.12.0 (android.jar
omits JDK-internal modules like `com.sun.net.httpserver`, so the Syncer
tests need a library for their real loopback server).

**Spec:** `docs/superpowers/specs/2026-07-11-android-client-design.md` -
read it first. The API contract is in
`docs/superpowers/specs/2026-07-10-are-you-up-design.md`.

**Conventions that apply to every task:** plain dash "-", never an em
dash. Commit messages: semantic title, wrapped prose, no co-author line.
Commands on a single line. The deployed backend for manual checks is
`http://100.88.181.84:8080`.

---

## File structure (locked in)

```
android/
  .gitignore additions go in the ROOT .gitignore (not a local one)
  Makefile
  README.md
  CLAUDE.md
  settings.gradle.kts
  build.gradle.kts
  gradle.properties
  gradlew, gradlew.bat, gradle/wrapper/*        (generated, committed)
  local.properties                              (NOT committed; sdk.dir)
  app/
    build.gradle.kts
    src/main/AndroidManifest.xml
    src/main/res/layout/main.xml
    src/main/java/dev/areyouup/
      core/Timestamps.kt        epoch ms -> RFC 3339 local-offset string
      core/Synthesizer.kt       events + cursor -> sample instants (pure)
      core/SampleQueue.kt       Sample data class + queue interface
      core/Store.kt             sqlite buffer, implements SampleQueue
      core/Syncer.kt            drain loop, POST, ack verification
      Prefs.kt                  SharedPreferences accessors (glue)
      SampleJob.kt              JobService + scheduling (glue)
      MainActivity.kt           status/config screen + event probe (glue)
    src/test/java/dev/areyouup/core/
      TimestampsTest.kt
      SynthesizerTest.kt
      StoreTest.kt              (Robolectric)
      FakeQueue.kt
      SyncerTest.kt             (MockWebServer loopback socket)
```

---

### Task 1: Gradle scaffold that builds an empty APK

**Files:**
- Create: `android/settings.gradle.kts`
- Create: `android/build.gradle.kts`
- Create: `android/gradle.properties`
- Create: `android/app/build.gradle.kts`
- Create: `android/app/src/main/AndroidManifest.xml`
- Create: `android/local.properties` (NOT committed)
- Modify: `.gitignore` (repo root)
- Generate: `android/gradlew`, `android/gradlew.bat`, `android/gradle/wrapper/*`

- [ ] **Step 1: Install the toolchain (skip pieces already present)**

Run each line separately; all are idempotent:

```bash
brew list --cask temurin@17 >/dev/null 2>&1 || brew install --cask temurin@17
brew list --cask android-commandlinetools >/dev/null 2>&1 || brew install --cask android-commandlinetools
brew list gradle >/dev/null 2>&1 || brew install gradle
yes | sdkmanager --licenses
sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0"
```

Expected: sdkmanager reports the packages installed (or already
installed). If `sdkmanager` is not on PATH, it lives under
`/opt/homebrew/share/android-commandlinetools/cmdline-tools/latest/bin/`.

- [ ] **Step 2: Write the Gradle files**

`android/settings.gradle.kts`:

```kotlin
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "are-you-up-android"
include(":app")
```

`android/build.gradle.kts`:

```kotlin
plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
```

`android/gradle.properties`:

```properties
org.gradle.jvmargs=-Xmx2g
# No androidx at runtime (framework-only, see android/CLAUDE.md), but
# Robolectric drags androidx.test artifacts onto the test classpath and
# AGP refuses any androidx artifact unless this flag is set.
android.useAndroidX=true
```

`android/app/build.gradle.kts`:

```kotlin
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "dev.areyouup"
    compileSdk = 34

    defaultConfig {
        applicationId = "dev.areyouup"
        minSdk = 34
        targetSdk = 34
        versionCode = 1
        versionName = "0.1"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.14.1")
    // Real org.json for JVM unit tests: the mockable android.jar ships
    // non-functional org.json stubs. On the device the framework
    // implementation is used; this artifact never ships in the APK.
    testImplementation("org.json:json:20240303")
}
```

`android/app/src/main/AndroidManifest.xml` (permissions land now;
components are added by the tasks that create their classes):

```xml
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools">

    <uses-permission android:name="android.permission.INTERNET" />
    <!-- Special permission: the owner grants it once via
         Settings > Apps > Special app access > Usage access. -->
    <uses-permission
        android:name="android.permission.PACKAGE_USAGE_STATS"
        tools:ignore="ProtectedPermissions" />
    <!-- Required by JobInfo.setPersisted(); no boot receiver exists or
         is ever added (ADR-0007). -->
    <uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED" />

    <!-- usesCleartextTraffic: the server is plain http inside the
         tailnet (no TLS by design; the tailnet is the perimeter). -->
    <application
        android:label="are-you-up"
        android:usesCleartextTraffic="true">
    </application>
</manifest>
```

`android/local.properties` (do NOT commit; gitignored in Step 3):

```properties
sdk.dir=/opt/homebrew/share/android-commandlinetools
```

(If `echo $ANDROID_HOME` prints a different SDK path, use that instead.)

- [ ] **Step 3: Append android entries to the root `.gitignore`**

Append these lines to the repo-root `.gitignore`:

```
android/.gradle/
android/.kotlin/
android/build/
android/app/build/
android/local.properties
```

- [ ] **Step 4: Generate and commit the Gradle wrapper**

```bash
cd android && gradle wrapper --gradle-version 8.9
```

Expected: `BUILD SUCCESSFUL`; creates `gradlew`, `gradlew.bat`,
`gradle/wrapper/gradle-wrapper.jar`,
`gradle/wrapper/gradle-wrapper.properties`.

- [ ] **Step 5: Verify the empty project builds**

```bash
cd android && ./gradlew --console=plain assembleDebug
```

Expected: `BUILD SUCCESSFUL` and
`android/app/build/outputs/apk/debug/app-debug.apk` exists. First run
downloads Gradle 8.9 and dependencies; that is normal.

```bash
cd android && ./gradlew --console=plain test
```

Expected: `BUILD SUCCESSFUL` (no tests yet).

- [ ] **Step 6: Commit**

```bash
git add .gitignore android/settings.gradle.kts android/build.gradle.kts android/gradle.properties android/app/build.gradle.kts android/app/src/main/AndroidManifest.xml android/gradlew android/gradlew.bat android/gradle/
git commit -m "feat(android): gradle scaffold, framework-only app skeleton"
```

Verify `git status` does NOT list `android/local.properties`,
`android/.gradle/`, or `android/app/build/` (they must be ignored).

---

### Task 2: On-device event probe (REQUIRES THE USER + the Pixel 7)

The spec's design assumption - that `SCREEN_INTERACTIVE`,
`SCREEN_NON_INTERACTIVE`, `KEYGUARD_HIDDEN`, `KEYGUARD_SHOWN`, and
`DEVICE_SHUTDOWN` events appear in `UsageStatsManager.queryEvents` with
sensible timestamps once Usage Access is granted - must be verified on
the actual phone before the Synthesizer's mapping is trusted. The probe
is not throwaway: the dump button stays as a permanent debugging aid.

**Coordinate with the user:** this task needs the Pixel 7 connected over
USB with debugging enabled. If the phone is not available right now,
implement Tasks 3-8 first and run this probe together with Task 9; the
event mapping is isolated to `SampleJob.queryEvents` + the
`Synthesizer.Event.Kind` enum, so a surprise here stays cheap to fix.

**Files:**
- Create: `android/app/src/main/java/dev/areyouup/MainActivity.kt` (probe version)
- Create: `android/app/src/main/res/layout/main.xml` (probe version)
- Modify: `android/app/src/main/AndroidManifest.xml` (add activity)

- [ ] **Step 1: Write the probe activity**

`android/app/src/main/java/dev/areyouup/MainActivity.kt`:

```kotlin
package dev.areyouup

import android.app.Activity
import android.app.AppOpsManager
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Intent
import android.os.Bundle
import android.os.Process
import android.provider.Settings
import android.util.Log
import android.widget.Button
import android.widget.TextView

class MainActivity : Activity() {

    companion object {
        const val TAG = "are-you-up"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.main)

        findViewById<Button>(R.id.grant).setOnClickListener {
            startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
        }
        findViewById<Button>(R.id.dump).setOnClickListener { dumpRecentEvents() }
    }

    override fun onResume() {
        super.onResume()
        findViewById<TextView>(R.id.status).text =
            "usage access: ${if (hasUsageAccess()) "granted" else "NOT GRANTED"}"
    }

    private fun hasUsageAccess(): Boolean {
        val ops = getSystemService(AppOpsManager::class.java)
        val mode = ops.unsafeCheckOpNoThrow(
            AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), packageName
        )
        return mode == AppOpsManager.MODE_ALLOWED
    }

    // The spec's on-device probe, kept forever as a debugging aid: dump
    // the last 2h of screen/keyguard usage events to logcat.
    private fun dumpRecentEvents() {
        val usm = getSystemService(UsageStatsManager::class.java)
        val now = System.currentTimeMillis()
        val events = usm.queryEvents(now - 2 * 60 * 60 * 1000, now)
        val e = UsageEvents.Event()
        var n = 0
        while (events.hasNextEvent()) {
            events.getNextEvent(e)
            val name = when (e.eventType) {
                UsageEvents.Event.SCREEN_INTERACTIVE -> "SCREEN_INTERACTIVE"
                UsageEvents.Event.SCREEN_NON_INTERACTIVE -> "SCREEN_NON_INTERACTIVE"
                UsageEvents.Event.KEYGUARD_HIDDEN -> "KEYGUARD_HIDDEN"
                UsageEvents.Event.KEYGUARD_SHOWN -> "KEYGUARD_SHOWN"
                UsageEvents.Event.DEVICE_SHUTDOWN -> "DEVICE_SHUTDOWN"
                UsageEvents.Event.DEVICE_STARTUP -> "DEVICE_STARTUP"
                else -> continue
            }
            Log.i(TAG, "event $name at ${e.timeStamp}")
            n++
        }
        Log.i(TAG, "dump: $n screen/keyguard events in last 2h")
    }
}
```

`android/app/src/main/res/layout/main.xml`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent"
    android:layout_height="match_parent"
    android:orientation="vertical"
    android:padding="16dp">

    <TextView
        android:id="@+id/status"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:fontFamily="monospace" />

    <Button
        android:id="@+id/grant"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:text="Grant usage access" />

    <Button
        android:id="@+id/dump"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:text="Dump events to log" />
</LinearLayout>
```

In `android/app/src/main/AndroidManifest.xml`, insert inside
`<application>`:

```xml
        <activity android:name=".MainActivity" android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
```

- [ ] **Step 2: Build and install on the phone (user assists)**

Phone prep (user, once): Settings > About phone > tap "Build number" 7x;
Settings > System > Developer options > enable "USB debugging"; plug in
over USB; accept the debugging prompt.

```bash
cd android && ./gradlew --console=plain assembleDebug && adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.areyouup/.MainActivity
```

Expected: `Success` from adb install; the app opens showing
"usage access: NOT GRANTED".

- [ ] **Step 3: Run the probe (user assists)**

1. Tap "Grant usage access", enable are-you-up, go back. Status should
   now read "granted".
2. Ask the user to: turn the screen off, wait ~5s, turn it on WITHOUT
   unlocking, wait ~5s, turn it off again, then unlock normally, then
   power the screen off and unlock once more.
3. Tap "Dump events to log", then:

```bash
adb logcat -d -s are-you-up
```

Expected observations to verify (this is the actual acceptance check):
- `SCREEN_INTERACTIVE` / `SCREEN_NON_INTERACTIVE` pairs for each screen
  on/off, with plausible epoch-ms timestamps in order.
- `KEYGUARD_HIDDEN` at each unlock; `KEYGUARD_SHOWN` when the lock
  re-engages.
- The screen-on-without-unlock sequence shows SCREEN events but no
  `KEYGUARD_HIDDEN`.

- [ ] **Step 4: Record findings in LAB_NOTES.md**

Append a dated entry to `LAB_NOTES.md`: what sequence was performed,
which event types appeared with what ordering/latency, and any
deviation from the spec's assumption. If events do NOT behave as
assumed, STOP and escalate: the Synthesizer mapping (Task 4) and the
spec need revisiting before proceeding.

- [ ] **Step 5: Commit**

```bash
git add android/app/src/main/java/dev/areyouup/MainActivity.kt android/app/src/main/res/layout/main.xml android/app/src/main/AndroidManifest.xml LAB_NOTES.md
git commit -m "feat(android): usage-event probe activity, verified on the Pixel 7"
```

---

### Task 3: Timestamps

**Files:**
- Create: `android/app/src/main/java/dev/areyouup/core/Timestamps.kt`
- Test: `android/app/src/test/java/dev/areyouup/core/TimestampsTest.kt`

- [ ] **Step 1: Write the failing tests**

`android/app/src/test/java/dev/areyouup/core/TimestampsTest.kt`:

```kotlin
package dev.areyouup.core

import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.ZoneId

class TimestampsTest {

    // 1783764000000 ms = 2026-07-11T10:00:00Z (fixed instant used throughout)

    @Test
    fun formatsSummerPositiveOffset() {
        assertEquals(
            "2026-07-11T13:00:00+03:00",
            Timestamps.format(1_783_764_000_000L, ZoneId.of("Asia/Jerusalem"))
        )
    }

    @Test
    fun formatsWinterOffsetOfSameZone() {
        // 1768471200000 ms = 2026-01-15T10:00:00Z; Israel is +02:00 in January.
        // The offset must reflect the zone rules AT the instant (ADR-0004).
        assertEquals(
            "2026-01-15T12:00:00+02:00",
            Timestamps.format(1_768_471_200_000L, ZoneId.of("Asia/Jerusalem"))
        )
    }

    @Test
    fun formatsNegativeOffset() {
        assertEquals(
            "2026-07-11T06:00:00-04:00",
            Timestamps.format(1_783_764_000_000L, ZoneId.of("America/New_York"))
        )
    }

    @Test
    fun formatsUtcAsZ() {
        // RFC 3339 allows Z for +00:00; the backend's chrono parser accepts it.
        assertEquals(
            "2026-07-11T10:00:00Z",
            Timestamps.format(1_783_764_000_000L, ZoneId.of("UTC"))
        )
    }

    @Test
    fun truncatesSubSecondPrecision() {
        assertEquals(
            "2026-07-11T13:00:00+03:00",
            Timestamps.format(1_783_764_000_999L, ZoneId.of("Asia/Jerusalem"))
        )
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd android && ./gradlew --console=plain test
```

Expected: compilation FAILS with unresolved reference `Timestamps`.

- [ ] **Step 3: Write the implementation**

`android/app/src/main/java/dev/areyouup/core/Timestamps.kt`:

```kotlin
package dev.areyouup.core

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

// ADR-0004: every timestamp in the system is an RFC 3339 string carrying
// the device's local UTC offset, computed per instant so DST changes and
// travel produce the offset in effect at that moment. XXX renders
// +03:00-style offsets (and Z for UTC, which RFC 3339 also allows).
object Timestamps {
    private val formatter = DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ssXXX")

    fun format(epochMs: Long, zone: ZoneId = ZoneId.systemDefault()): String =
        Instant.ofEpochMilli(epochMs).atZone(zone).format(formatter)
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd android && ./gradlew --console=plain test
```

Expected: `BUILD SUCCESSFUL`, 5 tests passing.

- [ ] **Step 5: Commit**

```bash
git add android/app/src/main/java/dev/areyouup/core/Timestamps.kt android/app/src/test/java/dev/areyouup/core/TimestampsTest.kt
git commit -m "feat(android): RFC 3339 local-offset timestamp formatting"
```

---

### Task 4: Synthesizer (events + cursor -> sample instants)

This is the one real algorithm in the app. It is pure Kotlin: no android
imports, no I/O, no clock access (the caller passes `nowMs`).

**Files:**
- Create: `android/app/src/main/java/dev/areyouup/core/Synthesizer.kt`
- Test: `android/app/src/test/java/dev/areyouup/core/SynthesizerTest.kt`

- [ ] **Step 1: Write the failing tests**

`android/app/src/test/java/dev/areyouup/core/SynthesizerTest.kt`:

```kotlin
package dev.areyouup.core

import dev.areyouup.core.Synthesizer.Event.Kind.LOCKED
import dev.areyouup.core.Synthesizer.Event.Kind.SCREEN_OFF
import dev.areyouup.core.Synthesizer.Event.Kind.SCREEN_ON
import dev.areyouup.core.Synthesizer.Event.Kind.SHUTDOWN
import dev.areyouup.core.Synthesizer.Event.Kind.UNLOCKED
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SynthesizerTest {

    private val t0 = 1_783_764_000_000L // 2026-07-11T10:00:00Z

    private fun s(sec: Int) = t0 + sec * 1000L

    private fun cursor(interactive: Boolean = false) =
        Synthesizer.Cursor(t0, screenOn = interactive, unlocked = interactive)

    private fun ev(sec: Int, kind: Synthesizer.Event.Kind) =
        Synthesizer.Event(s(sec), kind)

    @Test
    fun unlockSessionEmitsGridPlusEndSample() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(12, UNLOCKED), ev(107, LOCKED)),
            nowMs = s(600)
        )
        // window [12s, 107s]: grid at 12, 42, 72, 102 plus the end at 107
        assertEquals(listOf(s(12), s(42), s(72), s(102), s(107)), r.sampleTimesMs)
        assertFalse(r.next.interactive)
        assertEquals(s(600), r.next.tsMs)
    }

    @Test
    fun screenOnWhileLockedEmitsNothing() {
        // checking lock-screen notifications is not "using the phone"
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(40, SCREEN_OFF)),
            nowMs = s(600)
        )
        assertTrue(r.sampleTimesMs.isEmpty())
        assertFalse(r.next.interactive)
    }

    @Test
    fun relightInsideLockDelayResumesWithoutKeyguardEvent() {
        // screen off then on again before the keyguard re-engages: no
        // KEYGUARD events fire, yet the second window must still open
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                ev(10, SCREEN_ON), ev(12, UNLOCKED),
                ev(20, SCREEN_OFF), ev(25, SCREEN_ON), ev(30, LOCKED)
            ),
            nowMs = s(600)
        )
        // windows [12,20] and [25,30]
        assertEquals(listOf(s(12), s(20), s(25), s(30)), r.sampleTimesMs)
    }

    @Test
    fun openWindowEmitsUpToNowAndCursorStaysInteractive() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(12, UNLOCKED)),
            nowMs = s(82)
        )
        assertEquals(listOf(s(12), s(42), s(72), s(82)), r.sampleTimesMs)
        assertTrue(r.next.interactive)
    }

    @Test
    fun continuationFromInteractiveCursor() {
        // previous run left a window open; this run has no new events
        val r = Synthesizer.synthesize(cursor(interactive = true), emptyList(), s(50))
        assertEquals(listOf(s(0), s(30), s(50)), r.sampleTimesMs)
        assertTrue(r.next.interactive)
    }

    @Test
    fun shutdownClosesWindowAndResetsState() {
        val r = Synthesizer.synthesize(
            cursor(interactive = true),
            listOf(ev(40, SHUTDOWN)),
            nowMs = s(600)
        )
        assertEquals(listOf(s(0), s(30), s(40)), r.sampleTimesMs)
        assertFalse(r.next.interactive)
    }

    @Test
    fun subPeriodWindowEmitsStartAndEnd() {
        // UNLOCKED before SCREEN_ON also occurs (device wake paths vary);
        // the window opens when BOTH bits are finally true
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, UNLOCKED), ev(11, SCREEN_ON), ev(15, SCREEN_OFF)),
            nowMs = s(600)
        )
        assertEquals(listOf(s(11), s(15)), r.sampleTimesMs)
    }

    @Test
    fun zeroLengthWindowEmitsOneSample() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(10, UNLOCKED), ev(10, LOCKED)),
            nowMs = s(600)
        )
        assertEquals(listOf(s(10)), r.sampleTimesMs)
    }

    @Test
    fun multipleWindowsInOneRun() {
        // The LOCKED at 46 mirrors the probe-observed device behavior: the
        // keyguard re-engages ~1s after screen-off (LAB_NOTES 2026-07-11).
        // Without it, unlocked would survive the gap and the second window
        // would correctly open at the bare SCREEN_ON (the relight rule).
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                ev(10, SCREEN_ON), ev(10, UNLOCKED), ev(45, SCREEN_OFF),
                ev(46, LOCKED),
                ev(200, SCREEN_ON), ev(201, UNLOCKED), ev(230, LOCKED)
            ),
            nowMs = s(600)
        )
        assertEquals(listOf(s(10), s(40), s(45), s(201), s(230)), r.sampleTimesMs)
    }

    @Test
    fun eventsAtOrBeforeCursorAreIgnored() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                Synthesizer.Event(t0 - 5000, SCREEN_ON),
                Synthesizer.Event(t0, UNLOCKED)
            ),
            nowMs = s(600)
        )
        assertTrue(r.sampleTimesMs.isEmpty())
        assertFalse(r.next.interactive)
    }

    @Test
    fun eventsAfterNowAreIgnored() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(12, UNLOCKED), ev(700, LOCKED)),
            nowMs = s(600)
        )
        // the LOCKED event lies beyond now; window closes at now instead
        assertEquals(listOf(s(12), s(42), s(72), s(102), s(132), s(162),
            s(192), s(222), s(252), s(282), s(312), s(342), s(372), s(402),
            s(432), s(462), s(492), s(522), s(552), s(582), s(600)),
            r.sampleTimesMs)
        assertTrue(r.next.interactive)
    }

    @Test
    fun noEventsNotInteractiveJustAdvancesCursor() {
        val r = Synthesizer.synthesize(cursor(), emptyList(), s(600))
        assertTrue(r.sampleTimesMs.isEmpty())
        assertEquals(Synthesizer.Cursor(s(600), screenOn = false, unlocked = false), r.next)
    }

    @Test
    fun redundantEventsDoNotSplitWindows() {
        // duplicate SCREEN_ON / UNLOCKED must not restart the grid
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                ev(10, SCREEN_ON), ev(10, UNLOCKED),
                ev(20, SCREEN_ON), ev(25, UNLOCKED),
                ev(70, LOCKED)
            ),
            nowMs = s(600)
        )
        assertEquals(listOf(s(10), s(40), s(70)), r.sampleTimesMs)
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd android && ./gradlew --console=plain test
```

Expected: compilation FAILS with unresolved reference `Synthesizer`.

- [ ] **Step 3: Write the implementation**

`android/app/src/main/java/dev/areyouup/core/Synthesizer.kt`:

```kotlin
package dev.areyouup.core

// ==========================================================================
// Sample synthesis from the system usage-event log
// ==========================================================================
//
// ADR-0007: instead of observing the screen live, the app periodically
// replays the screen/keyguard events Android records anyway and
// reconstructs "the owner was using the phone" windows after the fact.
// This object is that reconstruction: pure, clock-free (the caller passes
// nowMs), and the only real algorithm in the app.
object Synthesizer {

    // Matches the mac client's cadence; the server merges consecutive
    // same-state samples while gaps stay <= 90s (3x this period).
    const val SAMPLE_PERIOD_MS = 30_000L

    // ADR-0006: the phone counts as active while the screen is on AND the
    // keyguard is dismissed. The two bits are tracked separately because
    // they change independently: re-lighting the screen inside the lock
    // delay fires screen events but no keyguard events.
    data class Cursor(val tsMs: Long, val screenOn: Boolean, val unlocked: Boolean) {
        val interactive: Boolean get() = screenOn && unlocked
    }

    data class Event(val tsMs: Long, val kind: Kind) {
        enum class Kind { SCREEN_ON, SCREEN_OFF, UNLOCKED, LOCKED, SHUTDOWN }
    }

    data class Result(val sampleTimesMs: List<Long>, val next: Cursor)

    // Replays `events` (sorted ascending by tsMs; events at or before the
    // cursor, or after nowMs, are ignored) and returns the sample instants
    // for every interactive window inside (cursor.tsMs, nowMs], plus the
    // cursor for the next run. A window still open at nowMs emits samples
    // up to nowMs and stays "interactive" in the returned cursor, so the
    // next run continues it seamlessly; the duplicate boundary sample this
    // produces is absorbed by INSERT OR IGNORE locally and the upsert
    // server-side.
    fun synthesize(cursor: Cursor, events: List<Event>, nowMs: Long): Result {
        var screenOn = cursor.screenOn
        var unlocked = cursor.unlocked
        var windowStart = if (cursor.interactive) cursor.tsMs else -1L
        val samples = mutableListOf<Long>()

        for (e in events) {
            if (e.tsMs <= cursor.tsMs || e.tsMs > nowMs) continue
            val wasInteractive = screenOn && unlocked
            when (e.kind) {
                Event.Kind.SCREEN_ON -> screenOn = true
                Event.Kind.SCREEN_OFF -> screenOn = false
                Event.Kind.UNLOCKED -> unlocked = true
                Event.Kind.LOCKED -> unlocked = false
                Event.Kind.SHUTDOWN -> { screenOn = false; unlocked = false }
            }
            val isInteractive = screenOn && unlocked
            if (!wasInteractive && isInteractive) {
                windowStart = e.tsMs
            } else if (wasInteractive && !isInteractive) {
                emitGrid(samples, windowStart, e.tsMs)
                windowStart = -1L
            }
        }
        if (windowStart >= 0) emitGrid(samples, windowStart, nowMs)
        return Result(samples, Cursor(nowMs, screenOn, unlocked))
    }

    // Samples at start, start+30s, start+60s, ... plus one at end, so the
    // server-observed interval spans the full window (no extrapolation).
    private fun emitGrid(out: MutableList<Long>, startMs: Long, endMs: Long) {
        var t = startMs
        while (t < endMs) {
            out.add(t)
            t += SAMPLE_PERIOD_MS
        }
        out.add(endMs)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd android && ./gradlew --console=plain test
```

Expected: `BUILD SUCCESSFUL`, 18 tests passing (5 Timestamps + 13 Synthesizer).

- [ ] **Step 5: Commit**

```bash
git add android/app/src/main/java/dev/areyouup/core/Synthesizer.kt android/app/src/test/java/dev/areyouup/core/SynthesizerTest.kt
git commit -m "feat(android): synthesize samples from screen/keyguard events"
```

---

### Task 5: Sample, SampleQueue, and Store

**Files:**
- Create: `android/app/src/main/java/dev/areyouup/core/SampleQueue.kt`
- Create: `android/app/src/main/java/dev/areyouup/core/Store.kt`
- Test: `android/app/src/test/java/dev/areyouup/core/StoreTest.kt`

- [ ] **Step 1: Write the interface file (no test; it is a declaration)**

`android/app/src/main/java/dev/areyouup/core/SampleQueue.kt`:

```kotlin
package dev.areyouup.core

data class Sample(val ts: String, val idleS: Int)

// The syncer's view of the store. Exists so the drain loop is testable
// on the plain JVM with an in-memory fake; Store itself needs Robolectric
// (android sqlite classes).
interface SampleQueue {
    fun nextBatch(limit: Int): List<Sample>
    fun markSynced(tss: List<String>)
}
```

- [ ] **Step 2: Write the failing Store tests**

`android/app/src/test/java/dev/areyouup/core/StoreTest.kt`:

```kotlin
package dev.areyouup.core

import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class StoreTest {

    // name = null -> in-memory database, fresh per Store instance
    private fun store() = Store(RuntimeEnvironment.getApplication(), null)

    @Test
    fun insertIsIdempotentOnTs() {
        val s = store()
        s.insert("2026-07-11T10:00:00+03:00", 0)
        s.insert("2026-07-11T10:00:00+03:00", 0)
        assertEquals(1, s.unsyncedCount())
    }

    @Test
    fun nextBatchRespectsLimitAndTsOrder() {
        val s = store()
        s.insert("2026-07-11T10:00:30+03:00", 0)
        s.insert("2026-07-11T10:00:00+03:00", 0)
        s.insert("2026-07-11T10:01:00+03:00", 0)
        assertEquals(
            listOf("2026-07-11T10:00:00+03:00", "2026-07-11T10:00:30+03:00"),
            s.nextBatch(2).map { it.ts }
        )
    }

    @Test
    fun batchCarriesIdleSeconds() {
        val s = store()
        s.insert("2026-07-11T10:00:00+03:00", 7)
        assertEquals(listOf(Sample("2026-07-11T10:00:00+03:00", 7)), s.nextBatch(10))
    }

    @Test
    fun markSyncedRemovesFromUnsynced() {
        val s = store()
        s.insert("2026-07-11T10:00:00+03:00", 0)
        s.insert("2026-07-11T10:00:30+03:00", 0)
        s.markSynced(listOf("2026-07-11T10:00:00+03:00"))
        assertEquals(listOf("2026-07-11T10:00:30+03:00"), s.nextBatch(10).map { it.ts })
        assertEquals(1, s.unsyncedCount())
    }

    @Test
    fun pruneOnlyDeletesSyncedRowsOlderThanCutoff() {
        val s = store()
        s.insert("2026-07-01T10:00:00+03:00", 0) // old, synced -> pruned
        s.insert("2026-07-01T10:00:30+03:00", 0) // old, UNSYNCED -> must survive
        s.insert("2026-07-11T10:00:00+03:00", 0) // recent, synced -> survives
        s.markSynced(listOf("2026-07-01T10:00:00+03:00", "2026-07-11T10:00:00+03:00"))
        s.pruneSynced(olderThanTs = "2026-07-08T00:00:00+03:00")
        assertEquals(listOf("2026-07-01T10:00:30+03:00"), s.nextBatch(10).map { it.ts })
        val total = s.readableDatabase
            .rawQuery("SELECT COUNT(*) FROM samples", null)
            .use { c -> c.moveToFirst(); c.getInt(0) }
        assertEquals(2, total)
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cd android && ./gradlew --console=plain test
```

Expected: compilation FAILS with unresolved reference `Store`.
(First Robolectric run downloads an android-all jar; that is normal.)

- [ ] **Step 4: Write the implementation**

`android/app/src/main/java/dev/areyouup/core/Store.kt`:

```kotlin
package dev.areyouup.core

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

// Local buffer between synthesis and sync - the same schema and rules as
// the mac client's store. Rows live here until the server acks them;
// pruning only ever touches synced rows, so an unreachable server never
// costs data.
class Store(context: Context, name: String? = "client.db") :
    SQLiteOpenHelper(context, name, null, 1), SampleQueue {

    init {
        // No effect on in-memory databases (tests); WAL on the device.
        setWriteAheadLoggingEnabled(true)
    }

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            "CREATE TABLE samples(" +
                "ts TEXT PRIMARY KEY, " +
                "idle_s INTEGER NOT NULL, " +
                "synced INTEGER NOT NULL DEFAULT 0)"
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        // ponytail: no migrations - single-table schema at version 1, the
        // same stance as the other two parts (see SESSION.md).
    }

    fun insert(ts: String, idleS: Int) {
        writableDatabase.execSQL(
            "INSERT OR IGNORE INTO samples(ts, idle_s, synced) VALUES(?, ?, 0)",
            arrayOf(ts, idleS)
        )
    }

    override fun nextBatch(limit: Int): List<Sample> {
        val out = mutableListOf<Sample>()
        readableDatabase.rawQuery(
            // $limit is a Kotlin Int: interpolation is injection-safe here
            "SELECT ts, idle_s FROM samples WHERE synced = 0 ORDER BY ts LIMIT $limit",
            null
        ).use { c ->
            while (c.moveToNext()) out.add(Sample(c.getString(0), c.getInt(1)))
        }
        return out
    }

    override fun markSynced(tss: List<String>) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            for (ts in tss) {
                db.execSQL("UPDATE samples SET synced = 1 WHERE ts = ?", arrayOf(ts))
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    // TEXT comparison on ts is only approximately chronological across
    // offset changes - fine for housekeeping (documented stance shared
    // with the mac client), unsound for anything correctness-critical.
    fun pruneSynced(olderThanTs: String) {
        writableDatabase.execSQL(
            "DELETE FROM samples WHERE synced = 1 AND ts < ?",
            arrayOf(olderThanTs)
        )
    }

    fun unsyncedCount(): Int =
        readableDatabase.rawQuery("SELECT COUNT(*) FROM samples WHERE synced = 0", null)
            .use { c -> c.moveToFirst(); c.getInt(0) }
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd android && ./gradlew --console=plain test
```

Expected: `BUILD SUCCESSFUL`, 23 tests passing.

- [ ] **Step 6: Commit**

```bash
git add android/app/src/main/java/dev/areyouup/core/SampleQueue.kt android/app/src/main/java/dev/areyouup/core/Store.kt android/app/src/test/java/dev/areyouup/core/StoreTest.kt
git commit -m "feat(android): sqlite sample buffer with sync bookkeeping"
```

---

### Task 6: Syncer (drain loop, POST, ack verification)

**Files:**
- Create: `android/app/src/main/java/dev/areyouup/core/Syncer.kt`
- Test: `android/app/src/test/java/dev/areyouup/core/FakeQueue.kt`
- Test: `android/app/src/test/java/dev/areyouup/core/SyncerTest.kt`
- Modify: `android/app/build.gradle.kts` (add the MockWebServer test dependency)

- [ ] **Step 0: Add the test-server dependency**

In `android/app/build.gradle.kts`, add to the dependencies block after
the org.json line:

```kotlin
    // Real loopback HTTP server for Syncer tests: unit tests compile
    // against android.jar, which excludes JDK-internal modules like
    // com.sun.net.httpserver (jdk.httpserver was never Android API).
    testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
```

- [ ] **Step 1: Write the fake queue**

`android/app/src/test/java/dev/areyouup/core/FakeQueue.kt`:

```kotlin
package dev.areyouup.core

class FakeQueue(samples: List<Sample>) : SampleQueue {
    val pending = samples.toMutableList()
    val synced = mutableListOf<String>()

    override fun nextBatch(limit: Int): List<Sample> = pending.take(limit)

    override fun markSynced(tss: List<String>) {
        synced += tss
        pending.removeAll { it.ts in tss }
    }
}
```

- [ ] **Step 2: Write the failing Syncer tests**

`android/app/src/test/java/dev/areyouup/core/SyncerTest.kt`:

```kotlin
package dev.areyouup.core

import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

// Runs the real Syncer (real HttpURLConnection) against a real local
// HTTP server socket (MockWebServer) - no mocking of the transport.
// (com.sun.net.httpserver is not usable here: AGP compiles unit tests
// against android.jar, which omits JDK-internal modules.)
class SyncerTest {

    private var server: MockWebServer? = null
    private val requests = mutableListOf<String>()

    private fun startServer(handler: (callIndex: Int, body: String) -> Pair<Int, String>): String {
        val s = MockWebServer()
        s.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                // Parity with a path-routed server: anything but the
                // contract path is a 404, so a Syncer that POSTs to the
                // wrong path fails the ack check loudly.
                if (request.path != "/v1/samples") return MockResponse().setResponseCode(404)
                val body = request.body.readUtf8()
                val index: Int
                synchronized(requests) {
                    index = requests.size
                    requests.add(body)
                }
                val (code, response) = handler(index, body)
                return MockResponse().setResponseCode(code).setBody(response)
            }
        }
        s.start()
        server = s
        return "http://127.0.0.1:${s.port}"
    }

    @After
    fun tearDown() {
        server?.shutdown()
    }

    private fun accepting() = { _: Int, body: String ->
        val n = JSONObject(body).getJSONArray("samples").length()
        200 to """{"accepted": $n}"""
    }

    // unique, valid RFC 3339 timestamps: 30s apart starting 10:00:00
    private fun samples(n: Int) = (0 until n).map { i ->
        val total = i * 30
        Sample("2026-07-11T%02d:%02d:%02d+03:00".format(10 + total / 3600, (total / 60) % 60, total % 60), 0)
    }

    @Test
    fun drainsInBatchesOf1000AndMarksAllSynced() {
        val url = startServer(accepting())
        val queue = FakeQueue(samples(1500))
        val outcome = Syncer(url, "pixel").sync(queue)
        assertEquals(Syncer.Outcome.Ok(1500), outcome)
        assertTrue(queue.pending.isEmpty())
        assertEquals(1500, queue.synced.size)
        assertEquals(2, requests.size) // 1000 + 500
    }

    @Test
    fun sendsTheContractShape() {
        val url = startServer(accepting())
        Syncer(url, "pixel").sync(FakeQueue(listOf(Sample("2026-07-11T10:00:00+03:00", 4))))
        val req = JSONObject(requests.single())
        assertEquals("pixel", req.getString("source"))
        val sample = req.getJSONArray("samples").getJSONObject(0)
        assertEquals("2026-07-11T10:00:00+03:00", sample.getString("ts"))
        assertEquals(4, sample.getInt("idle_s"))
    }

    @Test
    fun emptyQueueSucceedsWithoutAnyRequest() {
        val outcome = Syncer("http://127.0.0.1:1", "pixel").sync(FakeQueue(emptyList()))
        assertEquals(Syncer.Outcome.Ok(0), outcome)
    }

    @Test
    fun trailingSlashServerUrlIsTolerated() {
        // A configured URL like "http://host:8080/" must not become the
        // path //v1/samples (permanent 404).
        val url = startServer(accepting())
        val outcome = Syncer("$url/", "pixel").sync(FakeQueue(samples(1)))
        assertEquals(Syncer.Outcome.Ok(1), outcome)
    }

    @Test
    fun non200LeavesRowsUnsynced() {
        val url = startServer { _, _ -> 500 to "boom" }
        val queue = FakeQueue(samples(3))
        assertTrue(Syncer(url, "pixel").sync(queue) is Syncer.Outcome.Failed)
        assertEquals(3, queue.pending.size)
        assertTrue(queue.synced.isEmpty())
    }

    @Test
    fun ackMismatchLeavesRowsUnsynced() {
        // a server (or middlebox) claiming fewer rows than sent
        val url = startServer { _, _ -> 200 to """{"accepted": 2}""" }
        val queue = FakeQueue(samples(3))
        assertTrue(Syncer(url, "pixel").sync(queue) is Syncer.Outcome.Failed)
        assertEquals(3, queue.pending.size)
    }

    @Test
    fun nonJson200LeavesRowsUnsynced() {
        // captive portals answer 200 to anything; a bare 200 is not an ack
        val url = startServer { _, _ -> 200 to "<html>welcome to hotel wifi</html>" }
        val queue = FakeQueue(samples(1))
        assertTrue(Syncer(url, "pixel").sync(queue) is Syncer.Outcome.Failed)
        assertEquals(1, queue.pending.size)
    }

    @Test
    fun connectionRefusedFailsGracefully() {
        val queue = FakeQueue(samples(1))
        val outcome = Syncer("http://127.0.0.1:1", "pixel").sync(queue)
        assertTrue(outcome is Syncer.Outcome.Failed)
        assertEquals(1, queue.pending.size)
    }

    @Test
    fun midDrainFailureKeepsEarlierProgress() {
        val url = startServer { index, body ->
            if (index == 0) {
                val n = JSONObject(body).getJSONArray("samples").length()
                200 to """{"accepted": $n}"""
            } else {
                500 to "boom"
            }
        }
        val queue = FakeQueue(samples(1500))
        val outcome = Syncer(url, "pixel").sync(queue)
        assertEquals(1000, (outcome as Syncer.Outcome.Failed).synced)
        assertEquals(500, queue.pending.size) // first batch marked, second kept
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cd android && ./gradlew --console=plain test
```

Expected: compilation FAILS with unresolved reference `Syncer`.

- [ ] **Step 4: Write the implementation**

`android/app/src/main/java/dev/areyouup/core/Syncer.kt`:

```kotlin
package dev.areyouup.core

import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

// POSTs unsynced samples in batches until the queue drains. Rows are
// marked synced ONLY after the server's ack ({"accepted": N}) equals the
// batch size: a bare 200 is not an ack (captive portals answer 200 to
// anything), and a falsely-marked row becomes permanent data loss once
// pruning runs. Same rule as the mac client.
class Syncer(private val serverUrl: String, private val source: String) {

    companion object {
        const val BATCH_LIMIT = 1000
        private const val TIMEOUT_MS = 30_000
    }

    sealed class Outcome {
        data class Ok(val synced: Int) : Outcome()
        data class Failed(val synced: Int, val reason: String) : Outcome()
    }

    fun sync(queue: SampleQueue): Outcome {
        var total = 0
        while (true) {
            val batch = queue.nextBatch(BATCH_LIMIT)
            if (batch.isEmpty()) return Outcome.Ok(total)
            val error = postVerified(batch)
            if (error != null) return Outcome.Failed(total, error)
            queue.markSynced(batch.map { it.ts })
            total += batch.size
        }
    }

    // Returns null on verified success, else the failure reason. Never
    // throws: the job's dumb-retry loop only needs to know it failed.
    private fun postVerified(batch: List<Sample>): String? {
        val body = JSONObject()
            .put("source", source)
            .put("samples", JSONArray(batch.map { sample ->
                JSONObject().put("ts", sample.ts).put("idle_s", sample.idleS)
            }))
            .toString()
        return try {
            // trimEnd: a trailing slash in the configured URL would make
            // the path //v1/samples - a permanent, puzzling 404.
            val conn = URL("${serverUrl.trimEnd('/')}/v1/samples").openConnection() as HttpURLConnection
            try {
                conn.requestMethod = "POST"
                conn.connectTimeout = TIMEOUT_MS
                conn.readTimeout = TIMEOUT_MS
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                conn.outputStream.use { it.write(body.toByteArray()) }
                if (conn.responseCode != 200) {
                    return "server returned status ${conn.responseCode}"
                }
                val response = conn.inputStream.use { it.readBytes().decodeToString() }
                val accepted = try {
                    JSONObject(response).getInt("accepted")
                } catch (e: Exception) {
                    return "unparseable ack: ${response.take(120)}"
                }
                if (accepted != batch.size) {
                    return "ack mismatch: accepted=$accepted, sent=${batch.size}"
                }
                null
            } finally {
                conn.disconnect()
            }
        } catch (e: Exception) {
            "request failed: ${e.message}"
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd android && ./gradlew --console=plain test
```

Expected: `BUILD SUCCESSFUL`, 33 tests passing.

- [ ] **Step 6: Commit**

```bash
git add android/app/src/main/java/dev/areyouup/core/Syncer.kt android/app/src/test/java/dev/areyouup/core/FakeQueue.kt android/app/src/test/java/dev/areyouup/core/SyncerTest.kt
git commit -m "feat(android): batch sync with ack verification"
```

---

### Task 7: Glue - Prefs, SampleJob, final MainActivity, manifest

No unit tests in this task: everything here is deliberately thin
platform wiring (the same rule as the mac AppKit shell). Verification is
a clean build plus Task 9's on-device smoke.

**Files:**
- Create: `android/app/src/main/java/dev/areyouup/Prefs.kt`
- Create: `android/app/src/main/java/dev/areyouup/SampleJob.kt`
- Modify: `android/app/src/main/java/dev/areyouup/MainActivity.kt` (replace with final version)
- Modify: `android/app/src/main/res/layout/main.xml` (replace with final version)
- Modify: `android/app/src/main/AndroidManifest.xml` (add service)

- [ ] **Step 1: Write Prefs**

`android/app/src/main/java/dev/areyouup/Prefs.kt`:

```kotlin
package dev.areyouup

import android.content.Context
import dev.areyouup.core.Synthesizer

// SharedPreferences accessors - the android-native equivalent of the mac
// client's config.json, plus the job's cursor and status breadcrumbs.
class Prefs(context: Context) {
    private val p = context.getSharedPreferences("are-you-up", Context.MODE_PRIVATE)

    var serverUrl: String
        get() = p.getString("server_url", "http://100.88.181.84:8080")!!
        set(v) { p.edit().putString("server_url", v).apply() }

    var source: String
        get() = p.getString("source", "pixel")!!
        set(v) { p.edit().putString("source", v).apply() }

    var paused: Boolean
        get() = p.getBoolean("paused", false)
        set(v) { p.edit().putBoolean("paused", v).apply() }

    // The synthesis cursor: the instant up to which samples exist, plus
    // the screen state at that instant. null means "never ran" - the
    // first run starts at now (no backfill, per the spec).
    var cursor: Synthesizer.Cursor?
        get() {
            val ts = p.getLong("cursor_ts", 0L)
            if (ts == 0L) return null
            return Synthesizer.Cursor(
                ts,
                screenOn = p.getBoolean("cursor_screen_on", false),
                unlocked = p.getBoolean("cursor_unlocked", false)
            )
        }
        set(v) {
            requireNotNull(v) { "cursor only ever advances, never resets" }
            p.edit()
                .putLong("cursor_ts", v.tsMs)
                .putBoolean("cursor_screen_on", v.screenOn)
                .putBoolean("cursor_unlocked", v.unlocked)
                .apply()
        }

    var lastRunSummary: String
        get() = p.getString("last_run", "never")!!
        set(v) { p.edit().putString("last_run", v).apply() }

    var lastSyncTs: String
        get() = p.getString("last_sync", "never")!!
        set(v) { p.edit().putString("last_sync", v).apply() }
}
```

- [ ] **Step 2: Write SampleJob**

`android/app/src/main/java/dev/areyouup/SampleJob.kt`:

```kotlin
package dev.areyouup

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.ComponentName
import android.content.Context
import android.util.Log
import dev.areyouup.core.Store
import dev.areyouup.core.Syncer
import dev.areyouup.core.Synthesizer
import dev.areyouup.core.Timestamps
import kotlin.concurrent.thread

// ==========================================================================
// The entire runtime of the app (ADR-0007)
// ==========================================================================
//
// A persisted 15-minute periodic job: replay system screen/keyguard
// events from the stored cursor, synthesize samples, buffer, sync,
// prune, exit. No other component of this app ever runs in the
// background - do not add receivers, services, alarms, or wakelocks.
class SampleJob : JobService() {

    companion object {
        const val TAG = "are-you-up"
        private const val JOB_ID = 1
        private const val PERIOD_MS = 15 * 60 * 1000L
        private const val PRUNE_AFTER_MS = 7 * 24 * 60 * 60 * 1000L

        // No constraints beyond the period: the job must run even with no
        // network (samples buffer; sync just fails and retries next run).
        fun schedule(context: Context) {
            val scheduler = context.getSystemService(JobScheduler::class.java)
            // Re-scheduling an existing periodic job resets its phase, so
            // only schedule when absent (e.g. first launch, or after a
            // force-stop cancelled it).
            if (scheduler.getPendingJob(JOB_ID) != null) return
            scheduler.schedule(
                JobInfo.Builder(JOB_ID, ComponentName(context, SampleJob::class.java))
                    .setPeriodic(PERIOD_MS)
                    .setPersisted(true) // survives reboot; needs RECEIVE_BOOT_COMPLETED
                    .build()
            )
            Log.i(TAG, "job scheduled: every ${PERIOD_MS / 60_000} min, persisted")
        }
    }

    // Jobs start on the main thread; sqlite + network work happens on a
    // worker thread that reports completion via jobFinished.
    override fun onStartJob(params: JobParameters): Boolean {
        thread(name = "are-you-up-job") {
            try {
                runOnce(applicationContext)
            } catch (e: Exception) {
                // Log-and-finish keeps the periodic schedule alive. No
                // usage is lost: the cursor only advances after synthesis
                // and insertion succeeded, so the next run replays.
                Log.e(TAG, "job failed: ${e.message}", e)
            }
            jobFinished(params, false)
        }
        return true // work continues on the worker thread
    }

    override fun onStopJob(params: JobParameters): Boolean = true // retry later

    private fun runOnce(context: Context) {
        val prefs = Prefs(context)
        // First run ever: start at the current instant - history before
        // install is not reported (spec: no backfill).
        val cursor = prefs.cursor
            ?: Synthesizer.Cursor(System.currentTimeMillis(), screenOn = false, unlocked = false)
        // Clamped against backward clock steps (NTP/carrier resync between
        // runs): now < cursor.tsMs would synthesize a spurious past sample
        // and regress the cursor (LAB_NOTES 2026-07-11). Clamping turns the
        // run into a no-op span instead; the next run heals naturally.
        val now = maxOf(System.currentTimeMillis(), cursor.tsMs)

        val events = queryEvents(context, cursor.tsMs, now)
        val result = Synthesizer.synthesize(cursor, events, now)

        val store = Store(context)
        try {
            if (prefs.paused) {
                // Paused spans become permanent gaps: drop the samples but
                // still advance the cursor (mac pause semantics).
                Log.i(TAG, "paused: dropped ${result.sampleTimesMs.size} samples")
            } else {
                for (t in result.sampleTimesMs) store.insert(Timestamps.format(t), 0)
            }
            prefs.cursor = result.next

            val outcome = Syncer(prefs.serverUrl, prefs.source).sync(store)
            val summary = when (outcome) {
                is Syncer.Outcome.Ok -> {
                    prefs.lastSyncTs = Timestamps.format(now)
                    store.pruneSynced(Timestamps.format(now - PRUNE_AFTER_MS))
                    "${Timestamps.format(now)}: ${events.size} events, " +
                        "${result.sampleTimesMs.size} samples, synced ${outcome.synced}"
                }
                is Syncer.Outcome.Failed ->
                    "${Timestamps.format(now)}: ${events.size} events, " +
                        "${result.sampleTimesMs.size} samples, " +
                        "sync FAILED after ${outcome.synced}: ${outcome.reason}"
            }
            prefs.lastRunSummary = summary
            Log.i(TAG, summary)
        } finally {
            store.close()
        }
    }

    // Maps the system's usage events to the Synthesizer's platform-free
    // event type. Without the Usage Access grant, queryEvents just
    // returns nothing: the job logs "0 events" and the activity shows
    // the missing grant.
    private fun queryEvents(context: Context, fromMs: Long, toMs: Long): List<Synthesizer.Event> {
        val usm = context.getSystemService(UsageStatsManager::class.java)
        val out = mutableListOf<Synthesizer.Event>()
        val events = usm.queryEvents(fromMs, toMs)
        val e = UsageEvents.Event()
        while (events.hasNextEvent()) {
            events.getNextEvent(e)
            val kind = when (e.eventType) {
                UsageEvents.Event.SCREEN_INTERACTIVE -> Synthesizer.Event.Kind.SCREEN_ON
                UsageEvents.Event.SCREEN_NON_INTERACTIVE -> Synthesizer.Event.Kind.SCREEN_OFF
                UsageEvents.Event.KEYGUARD_HIDDEN -> Synthesizer.Event.Kind.UNLOCKED
                UsageEvents.Event.KEYGUARD_SHOWN -> Synthesizer.Event.Kind.LOCKED
                UsageEvents.Event.DEVICE_SHUTDOWN -> Synthesizer.Event.Kind.SHUTDOWN
                else -> null
            }
            if (kind != null) out.add(Synthesizer.Event(e.timeStamp, kind))
        }
        return out.sortedBy { it.tsMs } // defensive; the API returns sorted
    }
}
```

- [ ] **Step 3: Replace MainActivity with the final version**

`android/app/src/main/java/dev/areyouup/MainActivity.kt` (full file,
replaces the probe version; the probe's dump button survives):

```kotlin
package dev.areyouup

import android.app.Activity
import android.app.AppOpsManager
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Intent
import android.os.Bundle
import android.os.Process
import android.provider.Settings
import android.util.Log
import android.widget.Button
import android.widget.EditText
import android.widget.Switch
import android.widget.TextView
import dev.areyouup.core.Store

// The only UI: status + config. Opening it is also what (re)arms the
// job - including after a force-stop, which cancels persisted jobs.
class MainActivity : Activity() {

    private lateinit var prefs: Prefs

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.main)
        prefs = Prefs(this)

        findViewById<EditText>(R.id.server_url).setText(prefs.serverUrl)
        findViewById<Button>(R.id.save).setOnClickListener {
            prefs.serverUrl =
                findViewById<EditText>(R.id.server_url).text.toString().trim().trimEnd('/')
            refresh()
        }
        findViewById<Switch>(R.id.paused).setOnCheckedChangeListener { _, checked ->
            prefs.paused = checked
        }
        findViewById<Button>(R.id.grant).setOnClickListener {
            startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
        }
        findViewById<Button>(R.id.dump).setOnClickListener { dumpRecentEvents() }

        SampleJob.schedule(this)
    }

    override fun onResume() {
        super.onResume()
        refresh()
    }

    private fun refresh() {
        findViewById<Switch>(R.id.paused).isChecked = prefs.paused
        val store = Store(this)
        val unsynced = try {
            store.unsyncedCount()
        } finally {
            store.close()
        }
        findViewById<TextView>(R.id.status).text = """
            usage access: ${if (hasUsageAccess()) "granted" else "NOT GRANTED"}
            last run: ${prefs.lastRunSummary}
            last successful sync: ${prefs.lastSyncTs}
            unsynced samples: $unsynced
        """.trimIndent()
    }

    private fun hasUsageAccess(): Boolean {
        val ops = getSystemService(AppOpsManager::class.java)
        val mode = ops.unsafeCheckOpNoThrow(
            AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), packageName
        )
        return mode == AppOpsManager.MODE_ALLOWED
    }

    // The spec's on-device probe, kept forever as a debugging aid: dump
    // the last 2h of screen/keyguard usage events to logcat.
    private fun dumpRecentEvents() {
        val usm = getSystemService(UsageStatsManager::class.java)
        val now = System.currentTimeMillis()
        val events = usm.queryEvents(now - 2 * 60 * 60 * 1000, now)
        val e = UsageEvents.Event()
        var n = 0
        while (events.hasNextEvent()) {
            events.getNextEvent(e)
            val name = when (e.eventType) {
                UsageEvents.Event.SCREEN_INTERACTIVE -> "SCREEN_INTERACTIVE"
                UsageEvents.Event.SCREEN_NON_INTERACTIVE -> "SCREEN_NON_INTERACTIVE"
                UsageEvents.Event.KEYGUARD_HIDDEN -> "KEYGUARD_HIDDEN"
                UsageEvents.Event.KEYGUARD_SHOWN -> "KEYGUARD_SHOWN"
                UsageEvents.Event.DEVICE_SHUTDOWN -> "DEVICE_SHUTDOWN"
                UsageEvents.Event.DEVICE_STARTUP -> "DEVICE_STARTUP"
                else -> continue
            }
            Log.i(SampleJob.TAG, "event $name at ${e.timeStamp}")
            n++
        }
        Log.i(SampleJob.TAG, "dump: $n screen/keyguard events in last 2h")
    }
}
```

Note: the `companion object { const val TAG }` from the probe version of
MainActivity is gone; `TAG` now lives on `SampleJob` (single owner).

- [ ] **Step 4: Replace the layout with the final version**

`android/app/src/main/res/layout/main.xml` (full file):

```xml
<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent"
    android:layout_height="match_parent"
    android:orientation="vertical"
    android:padding="16dp">

    <TextView
        android:id="@+id/status"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:fontFamily="monospace" />

    <EditText
        android:id="@+id/server_url"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:hint="server url"
        android:inputType="textUri" />

    <Button
        android:id="@+id/save"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:text="Save server url" />

    <Switch
        android:id="@+id/paused"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:text="Paused" />

    <Button
        android:id="@+id/grant"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:text="Grant usage access" />

    <Button
        android:id="@+id/dump"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:text="Dump events to log" />
</LinearLayout>
```

- [ ] **Step 5: Add the job service to the manifest**

In `android/app/src/main/AndroidManifest.xml`, insert inside
`<application>`, after the `<activity>` element:

```xml
        <service
            android:name=".SampleJob"
            android:permission="android.permission.BIND_JOB_SERVICE" />
```

- [ ] **Step 6: Verify everything still builds and tests pass**

```bash
cd android && ./gradlew --console=plain test assembleDebug
```

Expected: `BUILD SUCCESSFUL`, 33 tests passing, APK produced.

- [ ] **Step 7: Commit**

```bash
git add android/app/src/main/java/dev/areyouup/Prefs.kt android/app/src/main/java/dev/areyouup/SampleJob.kt android/app/src/main/java/dev/areyouup/MainActivity.kt android/app/src/main/res/layout/main.xml android/app/src/main/AndroidManifest.xml
git commit -m "feat(android): job service, prefs, and status screen"
```

---

### Task 8: Makefile, READMEs, CLAUDE.md, root doc updates

**Files:**
- Create: `android/Makefile`
- Create: `android/README.md`
- Create: `android/CLAUDE.md`
- Modify: `README.md` (repo root)
- Modify: `CLAUDE.md` (repo root)

- [ ] **Step 1: Write the Makefile**

`android/Makefile` (recipe lines are TABs, not spaces):

```make
# Thin aliases over gradlew/adb so commands stay uniform with backend/
# and mac/.
APK := app/build/outputs/apk/debug/app-debug.apk

.PHONY: build test install run log clean

build:
	./gradlew --console=plain assembleDebug

test:
	./gradlew --console=plain test

install: build
	adb install -r $(APK)

run: install
	adb shell am start -n dev.areyouup/.MainActivity

log:
	adb logcat -s are-you-up

clean:
	./gradlew --console=plain clean
```

- [ ] **Step 2: Write android/README.md**

```markdown
# are-you-up android client

Reports pixel screen-usage to the backend as source `"pixel"`. No
resident process: a persisted 15-minute `JobScheduler` job replays the
system's screen/keyguard usage events and synthesizes samples
retrospectively, so the app has no background battery/CPU/memory
footprint and no notification. Design:
`../docs/superpowers/specs/2026-07-11-android-client-design.md`
(ADR-0006, ADR-0007 in `../DECISIONS.md`).

## Prerequisites (once, on the dev machine)

    brew install --cask temurin@17 android-commandlinetools
    yes | sdkmanager --licenses
    sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0"
    echo "sdk.dir=/opt/homebrew/share/android-commandlinetools" > local.properties

(Adjust `sdk.dir` if `echo $ANDROID_HOME` says otherwise.)

## Build / test

    make build            # app/build/outputs/apk/debug/app-debug.apk
    make test             # JVM unit tests; no device needed

## Get it on the phone (once)

1. On the phone: Settings > About phone > tap "Build number" 7 times,
   then Settings > System > Developer options > enable "USB debugging".
2. Plug the phone in over USB; accept the "Allow USB debugging?" prompt.
3. `make run` (installs and launches).
4. In the app: tap "Grant usage access" and enable are-you-up; check
   the server URL; done. The job is now armed and survives reboots -
   the cable is no longer needed.

## Operate

    make log                                           # tail the app's logcat
    adb shell cmd jobscheduler run -f dev.areyouup 1   # force a job run now

The app screen shows the usage-access state, the last job run summary,
the last successful sync, and the unsynced sample count. "Paused" stops
sample synthesis (the paused span becomes a permanent gap); syncing of
already-buffered rows continues. "Dump events to log" writes the last
2h of raw screen/keyguard events to logcat for debugging.

E2E smoke: use the phone for a minute, force a job run, then

    curl "http://100.88.181.84:8080/v1/intervals?from=2026-07-11T00:00:00%2B03:00&to=2026-07-12T00:00:00%2B03:00&source=pixel"

(with today's dates) should show an `active` interval covering that
minute.

## Upgrade

    git pull && make install     # in-place; db, prefs, and cursor survive

## Troubleshooting

- "0 events" in every run summary: Usage Access is not granted (the
  app's status line says so too).
- Job never runs: open the app once - force-stop parks persisted jobs
  until the next launch.
- `INSTALL_FAILED_UPDATE_INCOMPATIBLE`: the APK was built on a machine
  with a different debug keystore. `adb uninstall dev.areyouup` first
  (buffered samples are lost; anything synced is already on the server).
```

- [ ] **Step 3: Write android/CLAUDE.md**

```markdown
# android client - notes for Claude

Kotlin app, single Gradle module, framework-only: NO third-party
runtime dependencies (hard constraint, mirrors `mac/`). Test-only
exceptions: Robolectric and `org.json:json` (the mockable android.jar
stubs org.json for JVM tests). Approved design:
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
```

- [ ] **Step 4: Update the root README.md**

In `README.md`, change the Layout line

```
- `android/` - later
```

to

```
- `android/` - Kotlin phone client (no resident process; 15-min job replays system usage events)
```

and in the Development section, after the `make -C mac test` line, add:

```
    make -C android test            # JVM unit tests: synthesizer, store, syncer, timestamps
```

- [ ] **Step 5: Update the root CLAUDE.md**

In `CLAUDE.md`, change

```
- `android/` - not started.
```

to

```
- `android/` - Kotlin phone client. Read `android/CLAUDE.md` before
  touching it.
```

and in the Commands block, after the `make -C mac test` line, add:

```
    make -C android test        # JVM unit tests, no device needed
```

- [ ] **Step 6: Verify the Makefile works**

```bash
cd android && make test
```

Expected: `BUILD SUCCESSFUL`, 33 tests passing.

- [ ] **Step 7: Commit**

```bash
git add android/Makefile android/README.md android/CLAUDE.md README.md CLAUDE.md
git commit -m "docs(android): makefile, readme, claude notes, root doc updates"
```

---

### Task 9: On-device E2E smoke (REQUIRES THE USER + the Pixel 7)

No new code. This validates the whole pipeline against the real phone
and the real backend, mirroring `scripts/e2e.sh` for the mac client.
If Task 2 was deferred, run its probe steps first.

**Files:**
- Modify: `LAB_NOTES.md` (append findings)

- [ ] **Step 1: Install and arm (user assists)**

```bash
cd android && make run
```

In the app: grant Usage Access (if not already), confirm the server URL
is `http://100.88.181.84:8080`, confirm status shows
"usage access: granted".

- [ ] **Step 2: Generate usage and force a run (user assists)**

1. Ask the user to unlock the phone and use it for ~2 minutes, then lock
   it.
2. Force the job:

```bash
adb shell cmd jobscheduler run -f dev.areyouup 1
adb logcat -d -s are-you-up
```

Expected: a summary line like
`<ts>: N events, M samples, synced M` with M >= 4 (2 minutes at 30s
grid) and synced == M.

- [ ] **Step 3: Verify server-side derivation**

```bash
curl -sf "http://100.88.181.84:8080/v1/intervals?from=$(date +%Y-%m-%d)T00:00:00%2B03:00&to=$(date -v+1d +%Y-%m-%d)T00:00:00%2B03:00&source=pixel"
```

Expected: JSON with at least one `{"source": "pixel", ..., "state":
"active"}` interval whose start/end bracket the usage window from
Step 2. (Adjust the `%2B03:00` offset if the machine's zone differs.)

- [ ] **Step 4: Verify the app status screen**

Reopen the app. Expected: "last run" shows the forced run's summary,
"last successful sync" shows a recent timestamp, "unsynced samples: 0".

- [ ] **Step 5: Record the E2E in LAB_NOTES.md and commit**

Append a dated entry: what was run, the summary line observed, the
interval returned by the server, and any surprises (event ordering,
job latency, sample counts).

```bash
git add LAB_NOTES.md
git commit -m "docs: record android on-device E2E findings"
```

---

## Plan self-review (performed at write time)

- **Spec coverage:** footprint requirement -> Tasks 7 (job-only runtime)
  and the ADR-0007 invariants in Task 8's CLAUDE.md; probe -> Task 2;
  synthesis rules -> Task 4; store/sync rules incl. ack + drain + prune
  -> Tasks 5-6; config/UI list -> Task 7; build/install/operate docs +
  Makefile -> Task 8; manual E2E -> Task 9. Timestamps/ADR-0004 ->
  Task 3.
- **Type consistency:** `Sample(ts, idleS)`, `SampleQueue.nextBatch/
  markSynced`, `Synthesizer.Cursor/Event/Result/synthesize`,
  `Syncer.Outcome.Ok/Failed`, `Store.insert/pruneSynced/unsyncedCount`,
  `Prefs.cursor/serverUrl/source/paused/lastRunSummary/lastSyncTs`,
  `SampleJob.TAG/schedule` - names match across all tasks.
- **Placeholders:** none; every code step contains the full file or the
  exact insertion.
