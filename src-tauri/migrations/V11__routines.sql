CREATE TABLE routines (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  name TEXT NOT NULL COLLATE NOCASE,
  task TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('once', 'daily', 'hourly', 'weekly')),
  at_time TEXT NOT NULL,
  on_date TEXT,
  days INTEGER NOT NULL DEFAULT 127,
  every_hours INTEGER,
  weekday INTEGER,
  active INTEGER NOT NULL DEFAULT 1,
  done INTEGER NOT NULL DEFAULT 0,
  next_run_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (agent_id, name)
);

CREATE INDEX routines_due ON routines(next_run_at) WHERE next_run_at IS NOT NULL;
