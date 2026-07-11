# are-you-up android client

Reports pixel screen-usage to the backend as source `"pixel"`. No
resident process: a persisted 15-minute `JobScheduler` job replays the
system's screen/keyguard usage events and synthesizes samples
retrospectively, so the app has no background battery/CPU/memory
footprint and no notification. Design:
`../docs/superpowers/specs/2026-07-11-android-client-design.md`
(ADR-0006, ADR-0007 in `../DECISIONS.md`).

## Prerequisites (once, on the dev machine)

    # any JDK 17+ works; skip temurin if one is already installed
    # (the temurin cask needs interactive sudo for its .pkg)
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
