# Plan: issue #36, e2e windows must not take focus

## Goal

Running `bun run e2e` must not bring Shikigami to the front or take keyboard focus from the app the
user is working in. The shipped app and `app:dev` keep their normal behaviour.

## Decisions

- Everything lives behind the `e2e` Cargo feature, so release and dev builds cannot change.
- In `setup`, under `#[cfg(feature = "e2e")]` and only when `SHIKIGAMI_E2E_BACKGROUND=1`:
  - set the macOS activation policy to `Accessory` (`app.set_activation_policy(tauri::ActivationPolicy::Accessory)`),
    so the test app has no Dock icon and is not activated as the frontmost app;
  - do not call `set_focus`; the window stays visible.
- E2E-only Tauri config overlay `e2e/tauri.e2e.conf.json`, passed with `--config` in the `e2e`
  build command, sets the main window's `focus: false` (and keeps title and size). The main
  `tauri.conf.json` is not touched.
- `e2e/wdio.shared.ts` passes `SHIKIGAMI_E2E_BACKGROUND=1` in the app env for every config.
- If `Accessory` breaks WebDriver interaction or screenshots, fall back to the window `focus: false`
  overlay alone and report what failed. If the window must be raised for screenshots, do not use
  `set_focus`; use `show()` without activation, and report.
- Rejected: hiding the window (`visible: false`): screenshots and element interaction depend on a
  rendered window.

## Tests

- New script `e2e/check-focus.sh` (POSIX sh, bounded with `timeout`): records the frontmost process
  name (`osascript -e 'tell application "System Events" to get name of first process whose frontmost is true'`)
  before the run, starts `bun run e2e` in the background, samples the frontmost process every second
  until the run ends (wait on its PID), and fails if any sample is `shikigami`. Prints the samples.
  It is a manual check script, not part of `bun run check`.
- All existing specs still pass 3/3 and their screenshots still show the app.

## Manual check

- Run `e2e/check-focus.sh` 3 times while another app (for example Finder) is frontmost: 3/3 runs
  never report `shikigami` as frontmost.
- `bun run app:dev` once with temp data dirs: the window comes to the front as before. Stop it by PID.
- Confirm `strings` on the release binary has no `SHIKIGAMI_E2E_BACKGROUND`.

## Out of scope

Running e2e on CI.

## Done

`bun run check` passes; `bun run e2e` 3/3; the focus check 3/3; `app:dev` still takes focus.
