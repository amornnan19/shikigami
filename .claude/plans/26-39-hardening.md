# Plan: issues #26 and #39, review follow-ups for agent create and the turn runner

## Goal

Close the low findings left from the reviews of #16 and #17, so M1 ships without known small gaps
in validation, rollback, redaction and the child environment.

## Decisions

### #26 (agent create)

1. Rollback failure: if removing the just-created folder fails after a failed insert, the returned
   error message ends with " The folder <path> could not be removed." (code unchanged).
2. Client validation: the name field shows its problem under the field as soon as the user has
   typed: "Use 1 to 40 characters." or "The name cannot be empty." (the same rules as Rust).
3. Constraint mapping: only a UNIQUE violation on `agents.name` (extended code
   `SQLITE_CONSTRAINT_UNIQUE` and the message naming `agents.name`) maps to `name-taken`; other
   constraint errors map to `database-failed`.
4. Env in tests: `resolve_workspaces_root` (and the data-dir resolver if it has the same pattern)
   takes the override value as a parameter; tests call the pure function. No test calls
   `std::env::set_var` or `remove_var`.
5. Control characters: a name containing any Unicode control character (`char::is_control`) is
   refused with a new code `name-invalid`, message "The name cannot contain line breaks or control
   characters." Also mirrored in the UI.

### #39 (turn runner)

1. Child environment allowlist: the child gets only `HOME`, `USER`, `LOGNAME`, `SHELL`, `TMPDIR`,
   `LANG`, `LC_*`, `TERM`, plus the explicit `PATH` and the Anthropic key when the mode is api-key.
   Everything else from the parent is dropped (including `CLAUDE*`, `CLAUDE_CODE_*`, cloud
   credentials, tokens). Add to `CLAUDE.md` "Running agent CLIs": "Build an agent CLI's environment
   from an allowlist (HOME, USER, LOGNAME, SHELL, TMPDIR, LANG, LC_*, TERM, PATH); never pass the
   app's whole environment."
2. Keychain status in the error: "Could not read the saved API key (Keychain error <code>)." when a
   code is known; the code is not secret. Test the downcast from the keyring error.
3. Stderr trim: after trimming the front of the stderr buffer, drop everything up to and including
   the first newline, so a partial first line is never shown. Test with a key straddling the cut.
4. `redact_value` redacts JSON object keys as well as values. Test.
5. Late registration after `kill_all`: confirm the closing flag from #18 covers a runner that
   registers its group after `kill_all` ran (it must SIGKILL its own group and not advance); add the
   test if missing.

## Tests

As listed per item; each is a Rust test except #26 item 2 (e2e: type 41 characters and see the
message; screenshot).

## Done

`bun run check`; `bun run e2e` 3/3; every acceptance check in #26 and #39.

### Flaky test (found while landing #20)

- `turns::runner::tests::a_quiet_tool_that_ends_within_the_tool_limit_is_not_stalled` failed once
  in a full parallel `cargo test` run and passed 5/5 alone. Make it independent of machine load:
  derive its sleep and limits so the margin between "tool ends" and "tool limit" is at least 3x the
  idle limit, or drive the clock through the injected timeouts instead of wall time. Prove it with
  10 consecutive full `cargo test --lib` runs, all green. Audit the other timing-based runner tests
  for the same pattern and widen their margins the same way.
