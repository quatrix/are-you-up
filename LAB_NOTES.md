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
