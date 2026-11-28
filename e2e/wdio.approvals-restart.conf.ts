import { join } from "node:path";
import { fakeProviderDir } from "./fake-providers";
import { createConfig, dataRoot, groupsRoot, pickedFolder } from "./wdio.shared";

export const config = createConfig(
  ["approvals-restart.e2e.ts"],
  join(dataRoot, "approvals"),
  undefined,
  {
    SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
    SHIKIGAMI_GROUPS_DIR: groupsRoot,
    SHIKIGAMI_E2E_PICK_FOLDER: pickedFolder,
  },
  true,
);
