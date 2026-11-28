import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fakeProviderDir, prepareFakeProviders } from "./fake-providers";
import { createConfig, dataRoot, groupsRoot, pickedFolder, pickedParent } from "./wdio.shared";

const config = createConfig(["groups.e2e.ts"], join(dataRoot, "groups"), undefined, {
  SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
  SHIKIGAMI_GROUPS_DIR: groupsRoot,
  SHIKIGAMI_E2E_PICK_FOLDER: pickedFolder,
});
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  rmSync(groupsRoot, { recursive: true, force: true });
  rmSync(pickedParent, { recursive: true, force: true });
  mkdirSync(pickedFolder, { recursive: true });
  writeFileSync(join(pickedFolder, "keep.txt"), "kept\n");
  prepareFakeProviders();
};

export { config };
