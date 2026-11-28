CREATE TABLE routine_runs (
  id TEXT PRIMARY KEY,
  routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
  scheduled_for INTEGER,
  turn_id TEXT REFERENCES turns(id) ON DELETE SET NULL,
  queued_id TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('scheduled', 'manual', 'late')),
  outcome TEXT NOT NULL CHECK (outcome IN ('queued', 'running', 'completed', 'failed', 'stopped', 'interrupted', 'skipped', 'missed')),
  reason TEXT,
  created_at INTEGER NOT NULL,
  ended_at INTEGER
);

CREATE INDEX routine_runs_routine ON routine_runs(routine_id, created_at);

ALTER TABLE turns ADD COLUMN routine_id TEXT REFERENCES routines(id) ON DELETE SET NULL;
ALTER TABLE turns ADD COLUMN routine_name TEXT;
ALTER TABLE queued_messages ADD COLUMN routine_id TEXT REFERENCES routines(id) ON DELETE CASCADE;
ALTER TABLE queued_messages ADD COLUMN routine_name TEXT;
