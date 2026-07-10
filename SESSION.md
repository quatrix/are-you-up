- Syncer's dumb-retry policy treats every failure (network, non-200, ack
  mismatch) the same: leave the batch unsynced and retry on the next tick.
  A batch the server permanently rejects (e.g. a future 400 for malformed
  data) would therefore retry forever at the head of the unsynced queue,
  and pruneSynced never touches unsynced rows, so the db would grow
  without bound. When dumb-retry is revisited, treat 4xx as a permanent
  failure (drop or quarantine the batch) and 5xx/network errors as
  transient (keep retrying).
