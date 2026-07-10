# are-you-up

Tracks keyboard/mouse activity on my devices and serves it as
active/idle intervals, as a correction signal for whoop's time-in-bed
detection (sofa laptop sessions are not bed time).

- `mac/` - Swift menu-bar client: samples seconds-since-last-input every
  30s into local sqlite, syncs batches to the backend every 60s.
- `backend/` - Rust REST server: stores raw samples, derives intervals
  at query time (`threshold_s` is a query parameter, tunable forever).
- `android/` - later.

Design: `docs/superpowers/specs/2026-07-10-are-you-up-design.md`.
Decisions: `DECISIONS.md`. Findings: `LAB_NOTES.md`.

Timestamps are RFC 3339 with local offset everywhere. Communication is
plain HTTP over tailscale; there is no auth (single-user tailnet).
