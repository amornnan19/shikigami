# Shikigami

Shikigami is a macOS app that runs coding agents on this computer. Each agent works in its own
folder, under a sandbox, and can run on a schedule or hand work to another agent. Everything stays
on this Mac. The project is at an early stage.

## Requirements

- macOS 13 or later
- [Bun](https://bun.sh)
- [Rust](https://www.rust-lang.org/tools/install)
- Xcode command line tools (`xcode-select --install`)

## Commands

```sh
bun install          # install dependencies
bun run app:dev      # run the app in development mode
bun run app:build    # build the packaged app
bun run check        # lint, typecheck and run all tests
```

## Build output

`bun run app:build` writes the app and the disk image to `src-tauri/target/release/bundle/`:

- `macos/Shikigami.app`
- `dmg/Shikigami_<version>_<arch>.dmg`

## Open an unsigned app

The build is not signed. The first time you open it, macOS blocks it. Do one of these:

- Right-click `Shikigami.app` in Finder, choose **Open**, then choose **Open** again.
- Open **System Settings > Privacy & Security** and choose **Open Anyway** for Shikigami.

## License

PolyForm Noncommercial 1.0.0. See [LICENSE](LICENSE).
