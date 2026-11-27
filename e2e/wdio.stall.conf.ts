import { join } from "node:path";
import { fakeProviderDir, prepareFakeProviders } from "./fake-providers";
import { createConfig, dataRoot } from "./wdio.shared";

const config = createConfig(["stall.e2e.ts"], join(dataRoot, "stall"), undefined, {
  SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
  SHIKIGAMI_STALL_SECONDS: "3",
});
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  prepareFakeProviders();
};

export { config };
