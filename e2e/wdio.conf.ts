import { join } from "node:path";
import { createConfig, dataRoot } from "./wdio.shared";

export const config = createConfig("launch.e2e.ts", join(dataRoot, "empty"));
