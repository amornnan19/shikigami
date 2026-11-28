import { join } from "node:path";
import { fakeProviderDir, prepareFakeProviders } from "./fake-providers";
import { createConfig, dataRoot } from "./wdio.shared";

const config = createConfig(["routines.e2e.ts"], join(dataRoot, "routines"), undefined, {
  SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
  SHIKIGAMI_TZ: "Asia/Tokyo",
});
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  prepareFakeProviders();
};

export { config };
