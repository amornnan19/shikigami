import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";

const claudeRow = 'section[aria-label="Providers"] li:nth-child(1)';
const codexRow = 'section[aria-label="Providers"] li:nth-child(2)';

describe("providers", () => {
  it("shows the fake Claude Code as ready and Codex as not installed", async () => {
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready"));
    await expect($(claudeRow)).toHaveText(expect.stringContaining("9.9.9 (Fake Claude Code)"));
    await expect($(codexRow)).toHaveText(expect.stringContaining("Codex"));
    await expect($(codexRow)).toHaveText(expect.stringContaining("Not installed"));
    await expect($(codexRow)).toHaveText(expect.stringContaining("Install"));
    await browser.saveScreenshot(join(import.meta.dirname, "..", "artifacts", "providers.png"));
  });

  it("refreshes on demand", async () => {
    const refresh = $("button=Refresh");
    await refresh.click();
    await expect(refresh).toBeEnabled();
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready"));
  });
});
