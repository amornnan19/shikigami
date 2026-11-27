---
name: check-app
description: Run the Shikigami dev app by hand against temp data folders, bring its window to the front, screenshot it, and stop it cleanly by PID. Use to confirm a change in the real app, with real CLIs, outside the e2e suite.
---

# Check the real app

Never touch the user's real data: always use temp folders through the env overrides.

## Steps

1. Make sure port 1420 is free: `lsof -nP -iTCP:1420 -sTCP:LISTEN`. If a Vite server from this
   repo holds it, stop that PID (check `ps -o command -p <pid>` first).
2. Start the app in the background with temp folders and record the PID:

   ```sh
   sp=<scratchpad>; data=$(mktemp -d); ws=$(mktemp -d)
   SHIKIGAMI_DATA_DIR=$data SHIKIGAMI_WORKSPACES_DIR=$ws bun run app:dev > $sp/dev.log 2>&1 &
   echo $! > $sp/dev.pid
   ```

   Add other overrides the change needs (`SHIKIGAMI_PROVIDER_PATH`, `SHIKIGAMI_SECRET_STORE` only in
   e2e builds).
3. Wait on the log, not a fixed sleep: loop until `Running .*target/debug/shikigami` appears, then
   find the app PID with `pgrep -f "target/debug/shikigami$"`.
4. Screenshot: bring the window to the front, then capture its bounds.

   ```sh
   osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true"
   b=$(osascript -e "tell application \"System Events\" to tell (first process whose unix id is $pid) to get {position, size} of window 1" | tr -d ' ')
   screencapture -x -R "$b" $sp/<name>.png
   ```

   Open the PNG and look at it before reporting what it shows.
5. Stop by PID: the app PID, then the `bun run app:dev` PID from `dev.pid`, then any Vite PID still
   on port 1420. Never kill by name. Confirm the port is free and `ps -p <pid>` finds nothing.
6. Report the absolute screenshot paths and what each shows.
