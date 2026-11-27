import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";
import { setFakeClaudeVersion } from "../fake-providers";

const claudeCard = 'section[aria-label="Claude"]';

describe("claude section", () => {
  it("shows the fake Claude Code as ready, signed in, with the subscription chosen", async () => {
    await expect($(claudeCard)).toHaveText(expect.stringContaining("Ready"));
    await expect($(claudeCard)).toHaveText(expect.stringContaining("9.9.9 (Fake Claude Code)"));
    await expect($(claudeCard)).toHaveText(expect.stringContaining("Signed in"));
    await expect($("#auth-mode-subscription")).toHaveAttribute("aria-checked", "true");
    await expect($("#auth-mode-api-key")).toHaveAttribute("aria-checked", "false");
    await browser.saveScreenshot(join(import.meta.dirname, "..", "artifacts", "providers.png"));
  });

  it("refreshes on demand", async () => {
    const refresh = $("button=Refresh");
    await refresh.click();
    await expect(refresh).toBeEnabled();
    await expect($(claudeCard)).toHaveText(expect.stringContaining("Ready"));
  });

  it("shows the new version after Refresh", async () => {
    setFakeClaudeVersion("9.9.10 (Fake Claude Code)");
    const refresh = $("button=Refresh");
    await refresh.click();
    await expect($(claudeCard)).toHaveText(expect.stringContaining("9.9.10 (Fake Claude Code)"));
    await browser.saveScreenshot(
      join(import.meta.dirname, "..", "artifacts", "providers-refreshed.png"),
    );
  });
});
