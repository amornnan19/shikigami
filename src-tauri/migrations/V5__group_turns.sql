ALTER TABLE turns ADD COLUMN group_id TEXT REFERENCES groups(id) ON DELETE CASCADE;
ALTER TABLE messages ADD COLUMN group_id TEXT REFERENCES groups(id) ON DELETE CASCADE;
ALTER TABLE messages ADD COLUMN author_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL;

CREATE INDEX turns_group_started ON turns(group_id, started_at);
CREATE INDEX messages_group ON messages(group_id);

CREATE TABLE group_queue (
  id TEXT PRIMARY KEY,
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  addressed_agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  content TEXT NOT NULL,
  from_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
  root_message_id TEXT,
  hop INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (group_id, position)
);

CREATE INDEX group_queue_agent ON group_queue(addressed_agent_id);

CREATE TABLE agent_group_sessions (
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  provider_session_id TEXT,
  last_seen_rowid INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (agent_id, group_id)
);
