# Spec: sandbox and approval

- Milestone: M2 MVP: sandbox and approval
- Status: shipped (M2)

## Purpose

An agent is safe by default. It works inside its folder, cannot read the user's secrets, cannot reach
the network from the commands it runs, and asks before anything risky. The user answers each request
in the chat with Allow once or Deny, and every answer is kept. If any part of this cannot be
enforced, the agent does not run rather than run unprotected.

## User stories

1. As a user, I can let an agent work in its folder without watching it, knowing it cannot touch the
   rest of my Mac.
2. As a user, I see a request in the chat when an agent wants to do something risky, with what it
   wants to run and where, and I can allow it once or deny it.
3. As a user, I know that if I do not answer, nothing risky happens.
4. As a user, I can see afterwards what an agent asked for and what I answered.
5. As a user, I know an agent cannot read my keys, passwords, SSH or cloud credentials, other agents'
   folders or Shikigami's own data, even if I allow a command.
6. As a user, I know an agent does not use my own Claude Code setup: my personal instructions,
   memory, session history and shell configuration stay mine.

## Behaviour

### The agent's folder

- An agent's folder for a turn is its workspace in its own chat, and the group's folder in a group
  chat. Below, "the folder" means that folder.
- One access level exists for now, **Workspace**: every agent has it and it cannot be changed yet.
  Code that decides access already takes the lower of two levels for work one agent causes another to
  do (M4), so later levels slot in.

### What runs without asking

- Reading and searching files inside the folder.
- Creating, editing and deleting files inside the folder.
- Anything that only reads the agent's own conversation (planning, to-do lists, thinking).

### What asks first

- Every shell command, even a harmless one. It runs inside the sandbox even when allowed: it can
  write only inside the folder and the temporary folder, cannot read protected locations, and has no network.
- Fetching a web page or searching the web.
- Any other tool the agent's CLI offers that is not in the list above.

### Never allowed, even with an answer

- Reading protected locations: SSH and GPG keys, cloud and container credentials, package-registry and
  git credentials, the macOS keychains, browser profiles, Mail and Messages data, shell history and
  shell configuration files, other agents' workspaces, and Shikigami's data folder.
- Reading or listing anything outside the folder (for now it is refused, not asked about).
- Writing outside the folder and the temporary folder, and writing to `.claude`, `.git/hooks`,
  `.git/config`, `.husky`, `.envrc` and `.mcp.json` inside the folder.
- Network access from shell commands.
- Running a command outside the sandbox.
- A request for one of these is denied at once, without a card, and the agent is told why.

### Approval card

- A request appears in the chat as a card under the running turn: who asks (the agent), what (the
  command, or the tool and its target path or address), where (the folder), and the reason the agent
  gave if it gave one. Two buttons: **Allow once** and **Deny**.
- The turn waits while the card is open. The agent's sidebar row (and the group's row) shows that it
  needs the user.
- No answer within 10 minutes: the request is denied, and the card says it timed out.
- Stop on the turn, a stall, quitting the app, or the app closing: every open card is denied.
- After an answer the card stays in the transcript, showing Allowed or Denied (or Timed out) and the
  time.
- A denied request is reported to the agent as refused; the turn goes on.
- Several requests at once in one turn each get a card, answered independently.

### Record

- Every request and its outcome are kept with the turn: what was asked, the answer, who answered (the
  user, or the timeout), and when. They survive restarts and appear in the transcript.

### Keeping the user's own setup out

- An agent never loads the user's personal Claude Code instructions, memory, settings, hooks,
  plugins, MCP servers or output styles.
- The agent's Claude Code session files and memory live in Shikigami's data folder, not in the user's
  own Claude Code history.
- The agent's shell does not read the user's shell configuration (aliases, functions, prompts,
  environment set there).
- A saved API key reaches the agent's CLI so turns can authenticate, but is not visible to any
  command the agent runs.
- Sign-in still works: a subscription sign-in is done through Anthropic's own flow; if agents need a
  sign-in of their own, Shikigami shows the exact command to run and never handles the credentials.
- A saved API key reaches the CLI through a helper file that is deleted the first time it is read, so
  it is in no environment or setting the agent can see. A key the provider rejects ends the turn as a
  sign-in failure.
- The agent's shell is bash, so the shell configuration of the user's own shell is not read.
- Agents use their own Claude Code config folder inside Shikigami's data folder. With a Claude
  subscription, that folder needs a one-time sign-in; Settings shows the command, a copy button and
  Check again.

### Fail closed

- If the sandbox cannot start (unsupported system, missing component), the turn does not run; it ends
  failed with "The sandbox could not start, so this agent did not run." and the agent shows needs
  attention.
- A group folder under `/Volumes` or inside a protected location is refused when the group is made.
- Claude Code older than 2.1.314 is refused: the turn does not run and the agent shows needs attention.
- If the CLI does not apply the sandbox settings (they look ignored), the turn is stopped before any
  tool runs.
- A broken or unknown access setting is treated as the strictest behaviour.
- If Shikigami cannot show an approval card (for example the app is shutting down), the request is
  denied.

## Edge cases

- The folder is missing: the turn does not start (existing rule).
- A command tries to reach the network: it fails inside the sandbox; the agent sees the error.
- A request arrives while the user is in another chat: the sidebar marker shows where.
- A group turn: the card appears in the group chat; any group member's request is answered the same
  way.
- An agent asks for the same command again after a denial: it gets a new card.
- The app restarts with cards open: they were denied at quit and show as denied.
- Work one agent hands to another runs at the lower of the two levels (both Workspace for now).

## Safety

- A new agent writes and reads only inside its folder, cannot read secrets, and asks before risky
  actions.
- No answer to an approval means deny. A broken setting means the strictest mode.
- Nothing connects to the network except the agent provider and, when the user allows a web request,
  the address shown on the card.

## Acceptance checks

1. Asked to read `~/.ssh` (or another protected location), the agent cannot: no card appears and the
   agent reports it was refused.
2. Asked to create a file outside its folder, the agent cannot.
3. Asked to run a command, a card shows the command and folder; Allow once runs it, Deny refuses it.
4. An allowed command that tries to reach the network fails.
5. A card left unanswered for 10 minutes is denied and shows Timed out (testable with a shorter limit
   in test builds).
6. Stop while a card is open denies it and ends the turn.
7. After a restart, past cards show their outcome in the transcript.
8. A command run by the agent cannot print the saved API key, while turns still authenticate with it.
9. An agent prompted in English answers in English (the user's personal instructions are not loaded),
   and nothing new appears in the user's own Claude Code history after a turn.
10. `alias` in an allowed command shows none of the user's aliases.
11. When the sandbox cannot start, the turn fails with the sandbox message and nothing runs.
12. In a group chat, a request shows as a card in the group and works the same way.

## Out of scope

- Other access levels (asking less, full access) and per-agent extra folders.
- "Always allow" rules.
- Approvals for routines with nobody present (M3 decides; until then they are denied).
- Approvals from notifications, the menu bar or other devices.
