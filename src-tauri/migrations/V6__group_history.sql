CREATE TABLE turns_v6 (
  id TEXT PRIMARY KEY,
  agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'stopped', 'failed', 'interrupted')),
  error TEXT,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  group_id TEXT REFERENCES groups(id) ON DELETE CASCADE
);

INSERT INTO turns_v6 (rowid, id, agent_id, status, error, started_at, ended_at, group_id)
SELECT rowid, id, agent_id, status, error, started_at, ended_at, group_id FROM turns;

DROP TABLE turns;
ALTER TABLE turns_v6 RENAME TO turns;
CREATE INDEX turns_agent_started ON turns(agent_id, started_at);
CREATE INDEX turns_group_started ON turns(group_id, started_at);

CREATE TABLE messages_v6 (
  id TEXT PRIMARY KEY,
  agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
  turn_id TEXT REFERENCES turns(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'notice')),
  content TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  group_id TEXT REFERENCES groups(id) ON DELETE CASCADE,
  author_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
  seq INTEGER NOT NULL DEFAULT 0
);

INSERT INTO messages_v6 (rowid, id, agent_id, turn_id, role, content, created_at, group_id, author_agent_id, seq)
SELECT rowid, id, agent_id, turn_id, role, content, created_at, group_id, author_agent_id, rowid FROM messages;

DROP TABLE messages;
ALTER TABLE messages_v6 RENAME TO messages;
CREATE INDEX messages_agent_created ON messages(agent_id, created_at);
CREATE INDEX messages_group ON messages(group_id);

CREATE TABLE message_seq (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  value INTEGER NOT NULL
);

INSERT INTO message_seq (id, value) SELECT 1, COALESCE(MAX(seq), 0) FROM messages;

CREATE TRIGGER messages_assign_seq AFTER INSERT ON messages
BEGIN
  UPDATE message_seq SET value = value + 1 WHERE id = 1;
  UPDATE messages SET seq = (SELECT value FROM message_seq WHERE id = 1) WHERE rowid = NEW.rowid;
END;

ALTER TABLE agent_group_sessions RENAME COLUMN last_seen_rowid TO last_seen_seq;
