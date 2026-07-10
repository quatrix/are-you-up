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
