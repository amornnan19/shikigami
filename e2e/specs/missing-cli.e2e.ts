import { existsSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { workspacesRoot } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const claudeCard = 'section[aria-label="Claude"]';

describe("a Claude Code CLI that is not installed", () => {
  it("shows install guidance in the Claude section", async () => {
    await expect($(claudeCard)).toHaveText(expect.stringContaining("Not installed"));
    await expect($(claudeCard).$("button=Install")).toBeDisplayed();
    await browser.saveScreenshot(join(artifacts, "missing-cli-section.png"));
  });

  it("refuses to create an agent and leaves no folder behind", async () => {
    await $("button=New agent").click();
    await $("#agent-name").setValue("Test Agent");
    await $("button=Create").click();
    await expect($("#agent-form-error")).toHaveText("This provider is not installed.");
    await browser.saveScreenshot(join(artifacts, "missing-cli-create-refused.png"));
    await $("button=Cancel").click();
    await expect($('[role="dialog"]')).not.toBeDisplayed();
    await expect($$('aside[aria-label="Agents"] li')).toBeElementsArrayOfSize(0);
    expect(existsSync(join(workspacesRoot, "test-agent"))).toBe(false);
  });
});
