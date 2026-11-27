# Spec: agents, workspaces and chat

- Milestone: M1 MVP: agents and chat
- Status: draft

## Purpose

The user creates agents and talks to them. Each agent is a coding assistant that runs through an
agent CLI the user already has (Claude Code or Codex), works in its own folder, and keeps its
conversation across app restarts. This is the core every later milestone builds on.

## User stories

1. As a user, I can create an agent with a name, a picture, a provider and a model.
2. As a user, I can see all my agents in a list and open one to chat with it.
3. As a user, I can send a message and watch the reply appear as it is written.
4. As a user, I can see what the agent is doing (tool steps) without it cluttering the answer.
5. As a user, I can stop a reply that is running.
6. As a user, I can keep sending messages while the agent is busy; they wait in a queue I can see,
   edit and remove.
7. As a user, I can close and reopen the app and find my agents and conversations as I left them.
8. As a user, I can edit an agent's name, picture and model without losing its conversation.
9. As a user, I can delete an agent.
10. As a user, I am told clearly when a provider CLI is missing or not signed in, and what to do.

## Behaviour

### Agents

- An agent has: name (required, 1-40 characters, unique among agents), picture, provider, model,
  and a workspace folder. It also has a creation time.
- Picture: an initial-letter avatar on a colour the user picks from a fixed palette. Image upload
  is out of scope.
- Providers in M1: Claude Code and Codex. The create form lists only providers whose CLI is
  installed; others show as unavailable with the reason.
- Model: chosen from a short list per provider, with a provider default preselected. The user can
  also type a model name.
- The agent list shows each agent's picture, name and state: idle, working, or needs attention
  (last turn failed or was interrupted). The list is ordered by most recent activity.
- Editing name, picture or model takes effect from the next message. The conversation stays.
- Changing an agent's provider is out of scope for M1.
- Delete asks for confirmation. It removes the agent and its conversation from the app. The
  workspace folder is kept by default; the dialog offers a checkbox to also move the folder to the
  Trash. A working agent must be stopped before it can be deleted.

### Workspaces

- Each new agent gets its own folder: `~/Shikigami/agents/<folder-name>`, where the folder name is
  derived from the agent name (lower case, spaces to dashes, unsafe characters removed) and made
  unique with a number suffix if needed. The folder is created when the agent is created.
- Renaming an agent does not rename or move its folder.
- The agent's standing instructions live in an instruction file at the workspace root, in the file
  name that agent's provider reads by itself (Claude Code and Codex each have their own convention).
  Shikigami creates the file with a short default text when the agent is created.
- The agent settings show the instruction file's text with Edit and Show in Finder. Saving writes
  the file. If the file changed on disk since it was opened, Save warns and asks before
  overwriting.

### Chat

- Each agent has one conversation. Multiple threads are out of scope for M1.
- Sending a message starts a turn. The agent's reply streams in as it is written.
- Tool activity (commands run, files read or changed, searches) appears as compact steps inside the
  turn, collapsed by default and expandable. The final answer is shown in full below them.
- A Stop button is visible while a turn runs. Stop ends the turn; steps still open are marked
  stopped; the partial answer stays, marked as stopped.
- One turn runs at a time per agent. A message sent while a turn runs joins a visible queue under
  the input. Queued messages can be edited or removed before they start. When the turn ends, the
  next queued message starts automatically.
- The conversation is saved as it happens: user messages when sent, reply text and steps as they
  arrive.
- Long conversations open at the newest messages; older ones load when the user scrolls up.
- Markdown in replies is rendered (headings, lists, code blocks with copy, links). Links open in the
  default browser.

### Provider sessions

- Shikigami keeps the provider's own session for each agent, so the agent remembers the
  conversation between messages and after restarts, as far as the provider allows.
- If a provider session cannot be resumed, Shikigami starts a new one and shows a notice in the chat
  that the agent may have lost earlier context.

### Missing or signed-out provider

- At launch and when the create form opens, Shikigami checks each provider: installed or not, and
  signed in or not (as far as the CLI reports it).
- If a provider is missing, the user sees which CLI to install and a link to its official install
  page. If it is not signed in, the user sees the command to sign in, to run in their own terminal.
- An agent whose provider becomes unavailable shows needs attention, and its input explains why
  instead of sending.
- Shikigami uses the user's own CLI sign-in. It never asks for or stores the user's provider
  password. API keys are out of scope for M1.

## Edge cases

- **App quits or crashes during a turn:** on the next launch the turn is marked interrupted, its
  partial reply and steps stay visible, and queued messages remain queued and do not start by
  themselves. The user presses Resume to continue the queue.
- **Provider process exits unexpectedly:** the turn is marked failed with the error text the
  provider gave, if any. The agent shows needs attention until the next successful turn.
- **No output for 5 minutes:** the turn is ended as stalled and marked failed with "No response
  from the provider for 5 minutes."
- **Empty message:** Send is disabled.
- **Very long message:** accepted up to 100,000 characters; above that Send is disabled with the
  limit shown.
- **Workspace folder deleted outside the app:** the agent shows needs attention; the user can
  recreate the folder from the agent settings.
- **Two agents with the same name:** refused at create and rename.
- **Disk full or database error while saving:** the user sees an error; no partial agent is left
  behind on create.

## Safety

- M1 has no sandbox yet; that is M2. Until M2 lands, agents run with the provider's own default
  permission prompts left on. Shikigami must not pass any flag that skips the provider's own
  approvals or widens its permissions.
- Shikigami does not change the user's provider configuration files.

## Acceptance checks

1. Create an agent "Test Agent" with Claude Code: it appears in the list, and
   `~/Shikigami/agents/test-agent/` exists with the instruction file inside.
2. Send "Reply with the word pong": the reply streams in and contains "pong".
3. Ask the agent to list the files in its folder: a tool step appears collapsed; expanding it shows
   the command; the answer follows.
4. Start a long task and press Stop: the turn ends within 3 seconds and is marked stopped.
5. While a turn runs, send two more messages: both appear in the queue; edit one, remove the other;
   the edited one runs after the current turn.
6. Quit and reopen the app: the agent and the full conversation are there.
7. Quit the app during a running turn and reopen: the turn shows as interrupted; queued messages
   wait for Resume.
8. Rename the agent and change its colour and model: the next reply uses the new model; history is
   intact; the folder name is unchanged.
9. Edit the instruction file in the app, then ask the agent about its instructions: the answer
   reflects the edit.
10. Create a second agent with the name "Test Agent": refused.
11. Delete the agent without the checkbox: it disappears from the app and the folder remains.
12. With a provider CLI not found on PATH, the create form marks it unavailable with install
    guidance.
13. Repeat checks 1-6 with Codex.

## Out of scope

- Multiple threads per agent, branching, side questions.
- Attachments, images, voice.
- Changing provider, multiple accounts, API keys, backup models.
- Memory, skills, MCP servers.
- Sandbox and approvals (M2), routines (M3), agents messaging each other (M4).
