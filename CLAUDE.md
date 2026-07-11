# are-you-up - notes for Claude

Personal activity tracker feeding whoop sleep-detection correction.
The approved design lives at
`docs/superpowers/specs/2026-07-10-are-you-up-design.md` and is the
source of truth for the API contract between the parts.

## Layout

- `backend/` - Rust REST server. Read `backend/CLAUDE.md` before
  touching it.
- `mac/` - Swift menu-bar client. Read `mac/CLAUDE.md` before touching
  it.
- `android/` - not started.

## Cross-cutting rules

- Timestamps are RFC 3339 strings with the device's local UTC offset,
  everywhere (ADR-0004): API, both sqlite databases, logs. Never
  convert to unix seconds or normalize to UTC in storage or on the
  wire.
- The API contract (`POST /v1/samples`, `GET /v1/intervals`) is shared
  by both parts. Change it spec-first, then both sides and their tests
  in the same change; the client's ack check (`{"accepted": N}`) is
  part of the contract.
- `source` is a free-form device name; adding a device must never
  require a schema change.
- Record architecture decisions in `DECISIONS.md`, empirical findings
  in `LAB_NOTES.md`, and noticed-but-deferred issues in `SESSION.md`.

## Commands

    make -C backend test        # 19 tests
    make -C mac test            # 24 tests
    make -C backend smoke       # E2E against a real server process
    scripts/e2e.sh              # joint E2E: real client -> real server
