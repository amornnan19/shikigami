import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fakeProviderDir, prepareFakeProviders, scriptFakeStream } from "./fake-providers";
import { subagentStream } from "./subagent-stream";
import { createConfig, dataRoot, groupsRoot, pickedFolder, pickedParent } from "./wdio.shared";

const config = createConfig(["subagents.e2e.ts"], join(dataRoot, "subagents"), undefined, {
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
  writeFileSync(join(pickedFolder, "notes.txt"), "group notes\n");
  prepareFakeProviders();
  scriptFakeStream(1, subagentStream(7));
  scriptFakeStream(2, subagentStream(7));
};

export { config };
