# Session notes

- `backend`'s schema uses `CREATE TABLE IF NOT EXISTS` with no migration
  story: a database file created before a schema change (e.g. before the
  `idle_s >= 0` CHECK was added) never gains the new constraint, since the
  `CREATE TABLE` is a no-op once the table exists. Harmless before this
  project has shipped a database anyone depends on; would need a real
  migration mechanism (or at least an `ALTER TABLE` step in `open_db`) once
  a deployed `are-you-up.db` needs to survive a schema change.
