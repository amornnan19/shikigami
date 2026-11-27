import { chmodSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export const fakeProviderDir = join(root, "e2e", "artifacts", "fake-providers");
export const fakeClaudeVersionFile = join(fakeProviderDir, "claude-version");

export function setFakeClaudeVersion(version: string) {
  writeFileSync(fakeClaudeVersionFile, `${version}\n`);
}

export function prepareFakeProviders() {
  mkdirSync(fakeProviderDir, { recursive: true });
  setFakeClaudeVersion("9.9.9 (Fake Claude Code)");
  const fakeClaude = join(fakeProviderDir, "claude");
  writeFileSync(
    fakeClaude,
    `#!/bin/sh
case "$1" in
  --version) read -r version < "\${0%/*}/claude-version"; echo "$version" ;;
  auth) echo '{"loggedIn": true}' ;;
esac
`,
  );
  chmodSync(fakeClaude, 0o755);
}
