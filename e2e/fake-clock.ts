import { renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export const clockFile = join(root, "e2e", "artifacts", "fake-clock.txt");

export function setFakeClock(iso: string) {
  const staged = `${clockFile}.next`;
  writeFileSync(staged, String(Date.parse(iso)));
  renameSync(staged, clockFile);
}
