CREATE TABLE approvals (
  id TEXT PRIMARY KEY,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  tool TEXT NOT NULL,
  summary TEXT NOT NULL,
  detail TEXT NOT NULL,
  reason TEXT,
  target TEXT,
  status TEXT NOT NULL CHECK (status IN ('pending', 'allowed', 'denied', 'timed-out')),
  answered_by TEXT CHECK (answered_by IN ('user', 'timeout', 'stop', 'shutdown', 'relay')),
  created_at INTEGER NOT NULL,
  answered_at INTEGER
);

CREATE INDEX approvals_turn ON approvals(turn_id);
