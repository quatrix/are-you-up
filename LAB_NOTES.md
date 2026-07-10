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
