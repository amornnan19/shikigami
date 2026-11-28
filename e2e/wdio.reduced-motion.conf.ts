import { join } from "node:path";
import { fakeProviderDir, prepareFakeProviders, scriptFakeStream } from "./fake-providers";
import { subagentStream } from "./subagent-stream";
import { createConfig, dataRoot } from "./wdio.shared";

const config = createConfig(
  ["subagents-reduced.e2e.ts"],
  join(dataRoot, "reduced-motion"),
  undefined,
  {
    SHIKIGAMI_PROVIDER_PATH: fakeProviderDir,
    SHIKIGAMI_E2E_REDUCE_MOTION: "1",
  },
);
const prepare = config.onPrepare as () => void;
config.onPrepare = () => {
  prepare();
  prepareFakeProviders();
  scriptFakeStream(1, subagentStream(7));
};

export { config };
