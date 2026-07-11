# Session notes

- `backend`'s schema uses `CREATE TABLE IF NOT EXISTS` with no migration
  story: a database file created before a schema change (e.g. before the
  `idle_s >= 0` CHECK was added) never gains the new constraint, since the
  `CREATE TABLE` is a no-op once the table exists. Harmless before this
  project has shipped a database anyone depends on; would need a real
  migration mechanism (or at least an `ALTER TABLE` step in `open_db`) once
  a deployed `are-you-up.db` needs to survive a schema change.
- Syncer's dumb-retry policy treats every failure (network, non-200, ack
  mismatch) the same: leave the batch unsynced and retry on the next tick.
  A batch the server permanently rejects (e.g. a future 400 for malformed
  data) would therefore retry forever at the head of the unsynced queue,
  and pruneSynced never touches unsynced rows, so the db would grow
  without bound. When dumb-retry is revisited, treat 4xx as a permanent
  failure (drop or quarantine the batch) and 5xx/network errors as
  transient (keep retrying).
- The mac client's default `server_url` (http://127.0.0.1:8080) collided
  with an unrelated service already listening on this machine during the
  M6 smoke run (it answered 405). No data risk (the syncer requires the
  server's `{"accepted": N}` ack before marking anything synced), but the
  default port is worth changing when the real deployment address is
  chosen.
- `Synthesizer.synthesize` with `nowMs < cursor.tsMs` (wall clock stepped
  backward between job runs - NTP/carrier time correction) emits one
  spurious active sample at the past `nowMs` when the cursor is
  interactive, and regresses the cursor. Bounded and self-healing
  (level-based events make the overlap replay idempotent; dedupe absorbs
  re-emitted rows), but Task 7's SampleJob should clamp
  `now = max(now, cursor.tsMs)` - or skip the run - when the clock has
  regressed. See LAB_NOTES.md 2026-07-11 Task 4 probe entry.
