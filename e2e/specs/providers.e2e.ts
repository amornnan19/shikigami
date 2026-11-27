import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { setFakeClaudeVersion } from "../fake-providers";

const claudeRow = 'section[aria-label="Providers"] li:nth-child(1)';

describe("providers", () => {
  it("shows the fake Claude Code as ready and lists no other provider", async () => {
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready"));
    await expect($(claudeRow)).toHaveText(expect.stringContaining("9.9.9 (Fake Claude Code)"));
    await expect($$('section[aria-label="Providers"] li')).toBeElementsArrayOfSize(1);
    await browser.saveScreenshot(join(import.meta.dirname, "..", "artifacts", "providers.png"));
  });

  it("refreshes on demand", async () => {
    const refresh = $("button=Refresh");
    await refresh.click();
    await expect(refresh).toBeEnabled();
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready"));
  });

  it("shows the new version after Refresh", async () => {
    setFakeClaudeVersion("9.9.10 (Fake Claude Code)");
    const refresh = $("button=Refresh");
    await refresh.click();
    await expect($(claudeRow)).toHaveText(expect.stringContaining("9.9.10 (Fake Claude Code)"));
    await browser.saveScreenshot(
      join(import.meta.dirname, "..", "artifacts", "providers-refreshed.png"),
    );
  });
});
