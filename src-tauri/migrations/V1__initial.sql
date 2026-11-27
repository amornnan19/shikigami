CREATE TABLE agents (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
  color TEXT NOT NULL,
  provider TEXT NOT NULL CHECK (provider IN ('claude-code', 'codex')),
  model TEXT NOT NULL,
  workspace_path TEXT NOT NULL,
  provider_session_id TEXT,
  created_at INTEGER NOT NULL,
  last_activity_at INTEGER NOT NULL
);

CREATE TABLE turns (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'stopped', 'failed', 'interrupted')),
  error TEXT,
  started_at INTEGER NOT NULL,
  ended_at INTEGER
);
CREATE INDEX turns_agent_started ON turns(agent_id, started_at);

CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  turn_id TEXT REFERENCES turns(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'notice')),
  content TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX messages_agent_created ON messages(agent_id, created_at);

CREATE TABLE steps (
  id TEXT PRIMARY KEY,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  detail TEXT,
  status TEXT NOT NULL CHECK (status IN ('running', 'done', 'stopped', 'failed')),
  created_at INTEGER NOT NULL,
  UNIQUE (turn_id, position)
);

CREATE TABLE queued_messages (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  content TEXT NOT NULL,
  position INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (agent_id, position)
);
