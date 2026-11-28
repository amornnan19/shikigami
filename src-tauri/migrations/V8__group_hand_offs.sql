ALTER TABLE turns ADD COLUMN root_message_id TEXT;
ALTER TABLE turns ADD COLUMN hop INTEGER NOT NULL DEFAULT 0;
ALTER TABLE turns ADD COLUMN kind TEXT NOT NULL DEFAULT 'user';
ALTER TABLE turns ADD COLUMN from_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL;

CREATE INDEX turns_group_root ON turns(group_id, root_message_id);

ALTER TABLE group_queue ADD COLUMN kind TEXT NOT NULL DEFAULT 'user';
