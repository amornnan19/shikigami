import { rmSync } from "node:fs";
import { join } from "node:path";
import { fakeProviderDir, prepareFakeProviders } from "./fake-providers";
import { createConfig, dataRoot, revealLog, trashDir } from "./wdio.shared";

const config = createConfig(["agent-settings.e2e.ts"], join(dataRoot, "edit-delete"), undefined, {
  SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
  SHIKIGAMI_E2E_TRASH_DIR: trashDir,
  SHIKIGAMI_E2E_REVEAL_LOG: revealLog,
});
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  rmSync(trashDir, { recursive: true, force: true });
  rmSync(revealLog, { force: true });
  prepareFakeProviders();
};

export { config };
