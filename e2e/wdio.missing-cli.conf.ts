import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { createConfig, dataRoot } from "./wdio.shared";

const dataDir = join(dataRoot, "missing-cli");
const emptyProviderDir = join(dataDir, "bin");

const config = createConfig(["missing-cli.e2e.ts"], dataDir, undefined, {
  SHIKIGAMI_PROVIDER_PATH: emptyProviderDir,
});
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  mkdirSync(emptyProviderDir, { recursive: true });
};

export { config };
