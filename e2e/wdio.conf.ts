import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const appBinaryPath = join(root, "src-tauri", "target", "debug", "shikigami");

mkdirSync(join(root, "e2e", "artifacts"), { recursive: true });

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: [join(root, "e2e", "specs", "*.e2e.ts")],
  maxInstances: 1,
  capabilities: [{ browserName: "tauri" }],
  services: [["tauri", { driverProvider: "embedded", appBinaryPath }]],
  framework: "mocha",
  reporters: ["spec"],
  logLevel: "warn",
  mochaOpts: { timeout: 60000 },
};
