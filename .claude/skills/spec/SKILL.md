---
name: spec
description: Write or update a behaviour spec in docs/specs/ for a Shikigami feature, studying the OpenBot reference under the clean-room rule. Use when a milestone or issue needs a spec, when the user asks how a feature should behave, or before breaking a milestone into issues.
---

# Write a behaviour spec

A spec says what the user sees and can do. It never says how OpenBot implements it. This keeps the
clean-room line: the code side builds only from specs and issues.

## Steps

1. Pick the area and its milestone. One spec per area: `docs/specs/<area>.md`
   (for example `agents-and-chat.md`, `sandbox-and-approval.md`, `routines.md`, `agent-messaging.md`).
2. Study the reference behaviour. Delegate to a read-only agent (`Explore`) with this brief, in
   English:
   > Study `../openbot-reference` (docs first, source only to confirm). Report only features and
   > how they work for the user: what they see, can do, states, defaults, edge cases. No code,
   > file paths, function, class, type or table names, CSS, interface strings, prompts or schema.
   > Mark anything inferred.
3. Write the spec in your own words, in English, with these sections:
   - **Purpose**: one paragraph.
   - **User stories**: "As a user, I can ..." lines.
   - **Behaviour**: rules, states and transitions, defaults.
   - **Edge cases**: crashes, busy agents, missing tools, limits, bad input.
   - **Safety**: the safety defaults from `CLAUDE.md` that apply, and any stricter rule.
   - **Acceptance checks**: numbered, each one observable in the running app.
   - **Out of scope**: what this spec does not cover yet.
   Where Shikigami should differ from OpenBot (safer defaults, fewer features), say what Shikigami
   does. Do not describe OpenBot.
4. Clean-room check before showing it. The spec must not contain:
   - the word `openbot` or `OpenBot` outside one optional "Inspired by" line,
   - any path or file name from the reference,
   - camelCase, PascalCase or snake_case identifiers taken from the reference,
   - text copied from the reference's interface or prompts.
   Run `grep -niE 'openbot|src/|\.tsx?|\.rs' docs/specs/<area>.md` and fix each hit that is not
   ours.
5. Show the spec to the user and wait for approval. Do not commit without approval.

## Done

The user approved the spec, and its acceptance checks are concrete enough to become issues with the
`issue` skill.
