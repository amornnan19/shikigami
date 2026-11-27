# Plan: issue #14, store app data in a local SQLite database

## Goal

Give the Rust side one SQLite database, opened at startup and migrated with numbered SQL files, that
every later M1 issue writes agents, turns, messages, steps and the queue into. If it cannot be
opened, the UI shows an error screen instead of a broken app.

## Decisions

- Location: Tauri's app data directory for `com.amornnan.shikigami`
  (`~/Library/Application Support/com.amornnan.shikigami/`), file `shikigami.db`. The environment
  variable `SHIKIGAMI_DATA_DIR` overrides the directory (used by tests and e2e for isolation).
- Crates (exact pins, check licenses): `rusqlite` with the `bundled` feature (no dependency on the
  system SQLite), `refinery` with the `rusqlite` feature, `uuid` (v7), `serde`, `thiserror`.
- Access: one `Database` struct holding a `Mutex<rusqlite::Connection>`, stored in Tauri managed
  state. Rejected: a connection pool (single-user desktop app, no need).
- Pragmas on open: `journal_mode = WAL`, `foreign_keys = ON`, `busy_timeout = 5000`.
- Migrations: `src-tauri/migrations/V1__initial.sql` embedded with `refinery::embed_migrations!`.
  Refinery runs each migration in a transaction and records it.
- IDs are UUID v7 strings. Times are INTEGER milliseconds since the Unix epoch (UTC).
- Startup: `run()` opens and migrates the database in `setup`. On success it manages
  `Database`; on failure it manages an `AppStartupError(String)` and keeps running so the UI can
  show it. A Tauri command `app_status` returns `{ "status": "ok" }` or
  `{ "status": "error", "message": "..." }`. Rejected: panicking, which shows nothing to the user.
- UI: `App.tsx` calls `app_status` on load. While loading it shows nothing; on error it shows a
  full-window error screen with the title "Shikigami could not start", the message, and the data
  folder path; on ok it shows the existing placeholder.
- Module layout: `src-tauri/src/db/mod.rs` (open, pragmas, migrate, `Database`),
  `src-tauri/src/db/error.rs` (`DbError` with thiserror), `src-tauri/src/commands.rs` (`app_status`).

## Schema (V1)

```sql
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
```

Agent state (idle / working / needs attention) is derived from turns and workspace checks, not
stored.

## Tests (Rust, `cargo test`)

- A fresh database in a temp dir migrates to version 1 and has the five tables.
- Opening and migrating twice is a no-op (still version 1, no error).
- Foreign keys are on: inserting a turn for a missing agent fails.
- Deleting an agent cascades to its turns, messages, steps and queue.
- A file that is not a SQLite database makes `open` return an error (not panic).

## E2E

- Extend `e2e/` with a spec that starts the app with `SHIKIGAMI_DATA_DIR` pointing to a temp dir
  containing a garbage `shikigami.db`, and asserts the error screen title. Screenshot it.
- The existing launch spec runs with its own empty temp data dir, so it never touches the real one.

## Out of scope

Any data access commands beyond `app_status` (they arrive with #16 onward).

## Done

`bun run check` and `bun run e2e` pass; the e2e error-screen spec passes 3/3; the first launch with
an empty data dir creates `shikigami.db` at version 1.
