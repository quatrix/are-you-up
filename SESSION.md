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
  (Resolved 2026-07-11: Task 7's `SampleJob.runOnce` clamps
  `now = maxOf(System.currentTimeMillis(), cursor.tsMs)`.)
- Android `Syncer.postVerified` reads the response body with an
  unbounded `readBytes()`. A hostile or broken server streaming an
  enormous body would raise `OutOfMemoryError`, which - being an
  `Error`, not an `Exception` - escapes the catch and voids the
  "never throws" contract. Unreachable from the real backend (ack is
  ~20 bytes) and harmless to the mark-synced invariant (crash happens
  before marking), and the mac twin buffers unboundedly too; a bounded
  read (a valid ack fits in 4KB) would close it if ever revisited.
- Android `MainActivity`'s save button persists whatever survives
  `trim().trimEnd('/')` with no URL validation: saving an empty or
  garbage string stores it verbatim and every sync fails with "request
  failed" until corrected (visible in the status text, so recoverable,
  but a `Uri.parse` sanity check would fail at the moment of the typo
  instead). The EditText also keeps the un-normalized text the user
  typed while the stored value is trimmed - cosmetic mismatch.
