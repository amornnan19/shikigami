# Spec: group chats and mentions

- Milestone: M4 MVP: agents hand work to each other
- Status: shipped (M4)

## Purpose

Several agents work together in a group chat. The user mentions an agent to give it work, and an
agent can mention another agent to hand part of the work on. A group chat has its own folder, which
the user chooses (for example a code repository), so the agents work on the same files. Agents stay
separate from group chats: one agent can be a member of many groups and still has its own one-to-one
chat.

## User stories

1. As a user, I can create a group chat with a name, a folder I choose, and two or more member
   agents, one of which is the lead.
2. As a user, I can see group chats in the sidebar next to agents and open one.
3. As a user, I can write `@Name` in a group chat to give that agent work; a message without a
   mention goes to the lead.
4. As a user, I see every message with its author (me or an agent) in one transcript, in order.
5. As an agent, I can hand part of the work to another member by mentioning it in my reply, and the
   group continues with that member's answer.
6. As a user, I can stop the current work in a group, and runaway hand-offs stop by themselves.
7. As a user, I can rename a group, change its members and lead, and delete it.

## Behaviour

### Group chats

- A group chat has: name (1-40 characters, unique among group chats), a folder, members (at least
  two agents), and a lead (one of the members). Created time and last activity.
- Folder: chosen with the macOS folder picker. Default suggestion `~/Shikigami/groups/<folder-name>`
  (created if chosen). Any existing folder may be picked except the disk root, the home folder, the
  Shikigami data folder and any agent's own workspace folder.
- The group's folder is never deleted by Shikigami. Deleting a group removes its transcript from the
  app only.
- A member agent keeps its own one-to-one chat and workspace. Work in a group happens in the group's
  folder.

### Turns in a group

- One turn at a time per group, whichever agent it belongs to. Messages sent while a turn runs wait
  in the group's queue (same rules as a one-to-one chat: visible, editable, removable, Stop).
- An agent's turn in a group runs Claude Code in the group's folder. It keeps one Claude Code
  session per agent per group, separate from its one-to-one session.
- The agent gets, with each turn:
  - its own standing instructions (the text of its workspace `CLAUDE.md`), added to Claude Code's
    system prompt;
  - a short group briefing: the group name, the member list with each member's name, and the rule
    "to hand work to a member, start a line with @Name followed by the task";
  - the group messages it has not seen yet (messages from the user and other agents since its last
    turn in this group), then the message addressed to it.
- The group folder's own `CLAUDE.md` is read by Shikigami and given to the agent as text; Claude
  Code does not load it itself. The folder's project settings (permissions, hooks, environment)
  never apply. Agents stay isolated from the user's personal Claude Code configuration, as in
  one-to-one chats.
- The group details (the briefing and the folder's `CLAUDE.md`) are sent when an agent's group
  session starts or when the details change, and again after Claude Code compacts the session.
- An agent-level failure (a sign-in failure, a missing Claude Code CLI) holds that agent's queued
  work until the user presses Resume.

### Mentions

- In a user message, each `@Name` (case-insensitive, a member name) addresses that member. Several
  mentions address several members, who answer one after another in the order mentioned. A message
  with no member mention goes to the lead.
- The composer suggests member names after `@`.
- In an agent's final reply, a line that starts with `@Name` (a member other than itself) is a
  hand-off: the rest of that line, plus any following lines up to the next hand-off line, is the task
  for that member. The member's turn is queued right after the current one.
- A line addressed to the agent that handed the current task over is part of the answer, not a
  hand-off: that agent continues with the answer anyway.
- When a member finishes a task handed to it by another agent, the agent that handed it off gets a
  turn to continue, with the member's answer among its unseen messages.
- Hand-offs run depth first: a hand-off made by the member that answers runs before the next
  hand-off of the first agent, and each agent that handed work on continues for the one above.
- At most 8 mentioned members are addressed by one message.
- A mention of a name that is not a member is shown as plain text and does nothing.
- When a member cannot finish a task handed to it (a failure or a stop of that turn), the agent that
  handed it off still gets a turn to continue, with a notice.

### Limits

- At most 8 agent turns follow one user message (hand-offs included). The 9th is not started; the
  group shows "Hand-off limit reached. Mention an agent to continue." and waits for the user.
- An agent cannot hand work to the agent that just handed work to it more than twice in a row for
  the same user message (ping-pong guard); further hand-offs back are ignored with a notice.
- Stop ends the running turn and drops the queued hand-offs of that message.

### Permissions

- Work one agent causes another to do runs at the lower of the two agents' access levels. Until M2
  adds access levels, every agent has the same level, so this rule has no visible effect yet; it is
  built into the hand-off path so M2 only fills in the levels.
- Sign-in: each agent's turn uses that agent's sign-in mode (subscription or API key).

## Edge cases

- A member agent is deleted: its group messages stay and show its author as "Former member"; it
  leaves every group; a group left with one member shows a notice
  and cannot run until another member is added. The lead is reassigned to the first remaining
  member.
- The group folder is missing: the group shows needs attention with "The group folder is missing."
  and the composer is disabled.
- The app quits during a group turn: the turn is interrupted on relaunch; the group's queue and
  pending hand-offs wait for Resume, as in one-to-one chats.
- Two groups share a folder: allowed, with a warning in the create dialog ("Another group uses this
  folder."), because their turns can run at the same time.
- An agent is busy in its one-to-one chat while it is mentioned in a group: the group turn waits
  until that turn ends (an agent runs one turn at a time across all chats).

## Acceptance checks

1. Create a group "Team" with agents A (lead) and B and a chosen folder: it appears in the sidebar.
2. Send "hello" with no mention: A answers.
3. Send "@B list the files": B answers; the transcript shows the author of each message.
4. A reply from A containing "@B check the README" starts B's turn right after A's; then A gets a
   turn with B's answer among its unseen messages.
5. A chain that keeps handing off stops after 8 agent turns with the limit notice.
6. Stop during a hand-off chain ends the running turn and starts no further hand-off.
7. An agent's group session is separate from its one-to-one session (asking in the group "what did
   I tell you in private?" does not reveal the one-to-one conversation).
8. Deleting the group keeps its folder and removes it from the sidebar.
9. A turn in the group runs in the group's folder (a file listing shows the group folder's files).
10. Quit during a group turn and reopen: the turn shows interrupted; queued work waits for Resume.

## Subagents

- When an agent's turn runs subagents, they are shown as paper figures beside that agent's avatar,
  one per running subagent; the group header shows how many each member has out. The figures take
  the agent's colour. With reduced motion set on the Mac, a static stacked badge with the count is
  shown instead of moving figures.

## Out of scope

- Agents running in parallel inside one group (worktrees).
- Access levels (M2) and routines (M3).
