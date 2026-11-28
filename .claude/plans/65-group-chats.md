# Plan: issue #65, create and manage group chats

## Goal

Store group chats (name, folder, members, lead) and let the user create, list, edit and delete them.
Turns in groups come in #66; this issue gives groups a home in the data and the UI.

## Decisions

### Data (migration V4, follow the Database rules in `CLAUDE.md`)

```sql
CREATE TABLE groups (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
  folder_path TEXT NOT NULL,
  lead_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  last_activity_at INTEGER NOT NULL
);
CREATE TABLE group_members (
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  joined_at INTEGER NOT NULL,
  PRIMARY KEY (group_id, agent_id)
);
CREATE INDEX group_members_agent ON group_members(agent_id);
```

### Rust (new module `src-tauri/src/groups/`)

- `Group { id, name, folderPath, members: [{ agentId, name, color }], leadAgentId, createdAt,
  lastActivityAt, attention: Option<{ code, message }> }`. Attention codes: `folder-missing`
  ("The group folder is missing."), `too-few-members` ("Add another agent to use this group.").
- Commands: `list_groups`, `create_group({ name, folderPath?, memberIds, leadAgentId })`,
  `update_group(groupId, { name?, memberIds?, leadAgentId? })`, `delete_group(groupId)`,
  `pick_group_folder() -> Option<String>` (native folder picker through `tauri-plugin-dialog`,
  exact pin, check license; under the `e2e` feature, `SHIKIGAMI_E2E_PICK_FOLDER` returns that path
  without a dialog), `suggest_group_folder(name) -> String`.
- Validation (Rust is the authority):
  - name like agents (1-40 scalars, no control characters, unique case-insensitive) with codes
    `name-empty`, `name-too-long`, `name-invalid`, `name-taken`;
  - at least two distinct existing members (`members-too-few`), lead must be a member
    (`lead-not-member`);
  - folder: if `folderPath` is absent, use `<groups root>/<folder-name>` (groups root
    `~/Shikigami/groups`, absolute override `SHIKIGAMI_GROUPS_DIR`; folder name derived like agent
    folders) and create it; if given, it must be an absolute path to an existing directory, and must
    not be the disk root, the home folder, the Shikigami data folder, or equal to / inside / contain
    any agent's workspace folder (`folder-refused` with a reason). Canonicalise before comparing.
  - another group with the same canonical folder: allowed, `create_group` returns
    `sharedFolderWith: [groupName]` so the UI can warn.
- `delete_group` deletes rows only; never touches the folder.
- Agent deletion (existing `delete_agent`): in the same transaction, for each group the agent leads
  set the lead to the first remaining member by position (or NULL if none). Cascades remove the
  membership. `too-few-members` attention follows from the data.

### UI (`src/features/groups/`)

- Sidebar: "Agents" and "Group chats" headings; group rows show a stacked avatar pair and the name;
  a "New group" button.
- New group dialog: name, folder ("Choose folder…" button calling `pick_group_folder`; otherwise
  shows the suggested default path), members (checkbox list of agents), lead (select among chosen
  members). Shared-folder warning after create: "Another group uses this folder."
- Group view: header (name, member avatars, folder path), an empty transcript "No messages yet."
  (turns are #66), attention banner for the two codes, a gear button opening group settings
  (rename, members, lead, delete with confirmation "Delete <name>? Its messages are removed from
  Shikigami. The folder is kept.").
- TanStack Query `groups` query; invalidate on agent changes too.

## Tests

- Rust: migration V4 on a V3 database keeps agents and turns; validation codes; refused folders
  (root, home, data dir, agent workspace, inside one, containing one, symlink to one); default folder
  creation; shared-folder result; delete keeps the folder; agent deletion reassigns the lead and
  leaves `too-few-members`; list order by last activity.
- E2E (`SHIKIGAMI_E2E_PICK_FOLDER`, temp dirs): create agents A and B, create group "Team" with a
  picked folder, see it in the sidebar; rename it; delete it and the folder still exists; deleting
  agent B makes the group show "Add another agent to use this group."; screenshots.

## Done

`bun run check`; `bun run e2e` 3/3; spec checks 1 and 8.
