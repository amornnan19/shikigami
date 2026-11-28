import { join } from "node:path";
import { clockFile, setFakeClock } from "./fake-clock";
import { fakeProviderDir } from "./fake-providers";
import { createConfig, dataRoot } from "./wdio.shared";

const config = createConfig(
  ["routine-runs-restart.e2e.ts"],
  join(dataRoot, "routine-runs"),
  undefined,
  {
    SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
    SHIKIGAMI_TZ: "UTC",
    SHIKIGAMI_E2E_CLOCK_FILE: clockFile,
  },
  true,
);
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  setFakeClock("2026-11-30T12:00:00Z");
};

export { config };
