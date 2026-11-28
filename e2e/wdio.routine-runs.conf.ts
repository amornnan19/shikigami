import { join } from "node:path";
import { clockFile, setFakeClock } from "./fake-clock";
import { fakeProviderDir, prepareFakeProviders } from "./fake-providers";
import { createConfig, dataRoot } from "./wdio.shared";

const startClock = "2026-11-28T07:00:00Z";

const config = createConfig(["routine-runs.e2e.ts"], join(dataRoot, "routine-runs"), undefined, {
  SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
  SHIKIGAMI_TZ: "UTC",
  SHIKIGAMI_E2E_CLOCK_FILE: clockFile,
});
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  prepareFakeProviders();
  setFakeClock(startClock);
};

export { config };
