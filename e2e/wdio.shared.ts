import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const appBinaryPath = join(root, "src-tauri", "target", "debug", "shikigami");

mkdirSync(join(root, "e2e", "artifacts"), { recursive: true });

export function createConfig(
  specNames: string[],
  dataDir: string,
  databaseContent?: string,
  appEnv: Record<string, string> = {},
): WebdriverIO.Config {
  return {
    runner: "local",
    specs: specNames.map((specName) => join(root, "e2e", "specs", specName)),
    maxInstances: 1,
    capabilities: [{ browserName: "tauri" }],
    services: [
      [
        "tauri",
        { driverProvider: "embedded", appBinaryPath, env: { ...appEnv, SHIKIGAMI_DATA_DIR: dataDir } },
      ],
    ],
    framework: "mocha",
    reporters: ["spec"],
    logLevel: "warn",
    mochaOpts: { timeout: 60000 },
    onPrepare() {
      rmSync(dataDir, { recursive: true, force: true });
      mkdirSync(dataDir, { recursive: true });
      if (databaseContent !== undefined) {
        writeFileSync(join(dataDir, "shikigami.db"), databaseContent);
      }
    },
  };
}

export const dataRoot = join(root, "e2e", "artifacts", "data");
