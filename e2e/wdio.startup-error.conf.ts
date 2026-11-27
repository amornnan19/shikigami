import { join } from "node:path";
import { createConfig, dataRoot } from "./wdio.shared";

export const config = createConfig(
  ["startup-error.e2e.ts"],
  join(dataRoot, "corrupt"),
  "this is not a database file".repeat(50),
);
