# Shikigami milestones

Shikigami is a macOS app that runs coding agents on this computer. Each agent works in its own
folder, under a sandbox, and can run on a schedule or hand work to another agent. Everything stays
on this Mac.

## Clean-room rule

Shikigami is written from scratch. It is inspired by OpenBot, whose source is kept outside this
repository for reference only.

- **Spec side** may study OpenBot: use the app, read its documentation and its source. It writes
  specs in `docs/specs/` that describe behaviour the user sees: what happens, not how OpenBot does it.
  A spec contains no OpenBot code, file names, function names, class names, CSS, interface strings,
  prompts or database schema.
- **Code side** writes code only from the specs in this repository. It does not open OpenBot's source,
  and nothing is copied from it: no code, styles, text, icons or assets.
- Third-party libraries are used under their own licenses.

## Milestones

Each milestone ends with a working app and a commit that the user approves.

### M0. Foundation

- Decide the stack. It differs from OpenBot's (not Electron, SolidJS or Effect).
- `CLAUDE.md` with the clean-room rule, code style and the checks to run.
- Own `LICENSE`, chosen by the user.
- Done when: an empty app window opens on macOS from `dev` and from a packaged build.

### M1. Specs

Write one behaviour spec per v1 area, in `docs/specs/`:

1. Agents, workspaces and chat
2. Sandbox and command approval
3. Routines
4. Agents handing work to each other

Each spec lists user stories, states and edge cases, and the acceptance checks for its milestone.
Done when: the user approves all four specs.

### M2. Agents, workspaces and chat

- Create, rename and delete an agent. Each agent is tied to one workspace folder.
- Chat with an agent through an installed agent CLI (Claude Code first, then Codex).
- Stream replies, show tool activity, stop a running turn.
- Keep agents and chat history in a local SQLite database. They survive an app restart.
- Done when: the M2 acceptance checks from spec 1 pass in the running app.

### M3. Sandbox and command approval

- An agent writes only inside its workspace.
- An agent cannot read sensitive paths (SSH keys, cloud credentials, keychains, browser profiles).
- Risky actions wait for the user's approval. The user can allow once, allow always, or deny.
- Safe defaults for a new agent: workspace access only, approval on.
- Done when: the M3 acceptance checks from spec 2 pass, including attempts to read or write outside
  the allowed paths.

### M4. Routines

- Schedule work for an agent: once, daily, weekly, or a custom interval.
- Pause, resume, edit and delete a routine. A new or imported routine starts paused.
- Show run history and the result of each run. Handle a run missed while the Mac was asleep.
- Done when: the M4 acceptance checks from spec 3 pass, including a missed run.

### M5. Agents handing work to each other

- An agent can send a message or a task to another agent and get the reply.
- An agent cannot use another agent to gain wider permissions than its own.
- The user sees who asked whom, and what was done.
- Done when: the M5 acceptance checks from spec 4 pass, including a refused escalation.

### M6. Release v1

- Icon, README, About screen.
- A packaged, signed-for-local-use `.app`.
- End-to-end run of every acceptance check, three times.
- The app makes no network connection except to the agent providers and sites the agents use.
- Done when: all checks pass 3/3 and the user approves the release.

## Out of scope for v1

Teams and shared servers, mobile and watch apps, a web client, cloud accounts and billing, a
marketplace, Windows and Linux.
