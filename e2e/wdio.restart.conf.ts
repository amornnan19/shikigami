import { join } from "node:path";
import { fakeProviderDir } from "./fake-providers";
import { createConfig, dataRoot } from "./wdio.shared";

export const config = createConfig(
  ["restart.e2e.ts"],
  join(dataRoot, "empty"),
  undefined,
  { SHIKIGAMI_PROVIDER_PATH: fakeProviderDir },
  true,
);
