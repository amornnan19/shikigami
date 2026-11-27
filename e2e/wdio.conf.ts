import { join } from "node:path";
import { fakeProviderDir, prepareFakeProviders } from "./fake-providers";
import { createConfig, dataRoot } from "./wdio.shared";

const config = createConfig(
  ["launch.e2e.ts", "providers.e2e.ts"],
  join(dataRoot, "empty"),
  undefined,
  { SHIKIGAMI_PROVIDER_PATH: fakeProviderDir },
);
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  prepareFakeProviders();
};

export { config };
