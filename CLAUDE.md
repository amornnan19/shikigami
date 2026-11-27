# Shikigami

A macOS app that runs coding agents on this computer. Each agent works in its own folder, under a
sandbox, and can run on a schedule or hand work to another agent. Everything stays on this Mac.

The repository is public on GitHub (`amornnan19/shikigami`). Anything committed is public.

## Clean-room rule

Shikigami is written from scratch. It is inspired by OpenBot, whose source is kept outside this
repository at `../openbot-reference` for reference only. OpenBot is licensed PolyForm Noncommercial;
none of it may enter this repository.

- **Spec side** may study OpenBot (use the app, read its docs and source) and writes behaviour specs
  in `docs/specs/` with the `spec` skill. A spec says what the user sees and can do, never how
  OpenBot does it.
- **Code side** builds only from the specs and the GitHub issues. It does not open
  `../openbot-reference`, and copies nothing from it: no code, file or identifier names, styles,
  interface text, prompts, icons, assets or schema.
- How a feature is built is our own design, decided with the user.
- Third-party libraries are used under their own licenses. Check a new dependency's license before
  adding it.

## Stack

- Desktop shell: Tauri 2. Backend in Rust (`src-tauri/`): agent CLI processes, sandbox, scheduling,
  database.
- UI: React with TypeScript (`src/`), built with Vite. Package manager: Bun.
- Storage: SQLite on this Mac. The user's database is the source of truth.
- macOS only. No cloud services of our own.

Libraries and tools (reasons in `docs/decisions/0001-stack.md`):

- UI: shadcn/ui on Radix, Tailwind CSS, TanStack Query for data from Rust, Zustand for screen state.
- Rust: Tokio (async and child processes), rusqlite with refinery migrations, serde.
- Lint and format: Biome (TypeScript), rustfmt and clippy (Rust).
- Tests: Vitest (UI), `cargo test` (Rust), WebdriverIO with the Tauri embedded provider for end to
  end on macOS (test builds only).
- Business rules and the sandbox live in Rust. The UI never runs SQL or spawns processes.
- Check a dependency's license before adding it. Permissive licenses (MIT, Apache-2.0, BSD, ISC)
  are fine.

## Database

Migrations run as one grouped refinery run with `PRAGMA foreign_keys = OFF`. `Database::open` turns
foreign keys on only after the run and a clean `PRAGMA foreign_key_check`. SQLite ignores
`foreign_keys` inside a transaction, so a table rebuild with foreign keys on would cascade-delete
every child row. Never edit a migration that has been merged; add a new one.

## Running agent CLIs

A CLI that Shikigami starts gets an explicit PATH built from the provider search list; a
Finder-launched app only has /usr/bin:/bin:/usr/sbin:/sbin, and npm-installed CLIs need node on
PATH. Start each CLI in its own process group and kill the group on timeout or stop. Bound how much
output is read. Start each agent CLI isolated from the user's own configuration of that CLI: no
user-level settings, hooks, plugins, output styles or MCP servers. For Claude Code:
`--setting-sources project --strict-mcp-config --permission-mode manual --permission-prompts none`.
Keep the CLI's own sign-in working.

## Secrets

Secrets live only in the macOS Keychain. Decrypt a secret only when it is about to be used (a turn),
never on a status or list path. A failed read must never block turning the feature off or removing
it. The value never reaches the UI, logs, errors, the database or Debug output. Redact a secret passed
to a child from every text that reaches the database or an event: step input and output, reply
text, notices and errors.

## Planning

- Milestones live on GitHub (M0 Foundation ... M9). MVP is M0-M5.
- Work is tracked as GitHub issues in a milestone. Use the `issue` skill to break a milestone down.
- One issue per branch: `<issue-number>-<short-slug>`, from `main`.
- Plan an issue with `plan-issue`, review it with `review-branch`, land it with `ship-issue`.

## Skills

A task that repeats and must be done the same way every time becomes a project skill in
`.claude/skills/<name>/SKILL.md`. When a procedure comes up a second time, write or update its skill
instead of repeating it from memory, and add it to this table. Use the skill whenever its task comes
up.

| Skill | Use it to |
| --- | --- |
| `spec` | write a behaviour spec in `docs/specs/` |
| `issue` | break a milestone or spec into GitHub issues |
| `plan-issue` | write an issue's plan in `.claude/plans/` and hand it to the implementer |
| `review-branch` | have a pushed branch reviewed from git objects |
| `check-app` | run the real app with temp data, screenshot it, stop it by PID |
| `ship-issue` | verify, commit, push, open and merge the pull request |

## Safety defaults (product rules)

These come from the specs and hold for every feature:

- A new agent writes and reads only inside its workspace, cannot read secrets, and asks before
  risky actions.
- No answer to an approval means deny. A broken setting means the strictest mode.
- Work that one agent or a routine causes another agent to do runs at the lower of the two access
  levels.
- Nothing connects to the network except agent providers and the sites agents use.

## Git

- Conventional Commits: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `perf:`, `style:`.
  Lower case after the colon, imperative, no trailing period. Reference the issue (`Refs #12`,
  `Closes #12`) in the body.
- **Ask the user before every commit and every push.** Show what is in it first.
- Never force-push, never `--no-verify`, never rewrite pushed history.
- `main` is protected on GitHub: no direct pushes, every change lands through a pull request. No
  review approval is required, so a PR can be merged once its checks pass.
- Write everything in the repository in English: code, comments, docs, commit messages, issues.

## Checks

Run the narrowest check for what changed, one at a time, narrowest first:

1. `bunx biome check <changed files>`: lint and format check for the TypeScript files you changed.
   `bun run format` fixes the whole repository with Biome.
2. `bun run typecheck`: after any change in `src/`.
3. `bunx vitest run <test file>`: the one test file for the code you changed. `bun run test` runs all.
4. Rust changes in `src-tauri/`: `bun run rust:fmt`, then `bun run rust:lint` (clippy, warnings are
   errors), then `bun run rust:test`.
5. `bun run lint` checks the whole repository with Biome.
6. `bun run e2e` builds a debug binary with the `e2e` Cargo feature and drives the real app with
   WebdriverIO. Screenshots go to `e2e/artifacts/` (git ignores it). It takes about a minute. Run it
   when you change what the user sees or a flow, and before you ask to ship such an issue. The `e2e`
   feature is for test builds only: never enable it in `app:dev` or `app:build`.
7. `bun run check` runs every check above except `bun run e2e` (it is slow), in sequence. Run it
   before you ask to ship an issue.

The pre-commit hook (`.githooks/pre-commit`) runs automatically on every commit. It checks staged
files with Biome and, when a staged file is under `src-tauri/`, runs `cargo fmt --check` and
`cargo clippy`. It never changes files. `bun install` turns the hook on. Never bypass it: fix the
error and commit again.
