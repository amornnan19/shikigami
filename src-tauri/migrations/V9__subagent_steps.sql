ALTER TABLE steps ADD COLUMN parent_step_id TEXT REFERENCES steps(id) ON DELETE CASCADE;

CREATE INDEX steps_parent ON steps(parent_step_id);
