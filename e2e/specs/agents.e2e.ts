import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { workspacesRoot } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");

async function openDialog() {
  await $("button=New agent").click();
  await expect($('[role="dialog"]')).toBeDisplayed();
}

describe("agents", function () {
  this.timeout(300000);

  it("creates an agent with Claude Code and selects it", async () => {
    await openDialog();
    await $("#agent-name").setValue("Test Agent");
    await expect($("button=Create")).toBeEnabled();
    await browser.saveScreenshot(join(artifacts, "agents-dialog.png"));
    await $("button=Create").click();

    const row = $('aside[aria-label="Agents"] li:nth-child(1) button');
    await expect(row).toHaveText(expect.stringContaining("Test Agent"));
    await expect(row).toHaveAttribute("aria-current", "true");
    await expect($('section[aria-label="Agent"] h2')).toHaveText("Test Agent");
    await expect($('section[aria-label="Agent"]')).toHaveText(
      expect.stringContaining("No messages yet."),
    );
    await browser.saveScreenshot(join(artifacts, "agents-list.png"));

    const folder = join(workspacesRoot, "test-agent");
    expect(existsSync(join(folder, "CLAUDE.md"))).toBe(true);
    expect(readFileSync(join(folder, "CLAUDE.md"), "utf8")).toContain("# Test Agent");
    expect(existsSync(join(folder, "AGENTS.md"))).toBe(false);
  });

  it("refuses a duplicate name under the name field", async () => {
    await openDialog();
    await $("#agent-name").setValue("test agent");
    await $("button=Create").click();
    await expect($("#agent-name-error")).toHaveText(expect.stringContaining("already exists"));
    await expect($("#agent-name")).toHaveAttribute("aria-invalid", "true");
    await browser.saveScreenshot(join(artifacts, "agents-name-taken.png"));
    await $("button=Cancel").click();
    await expect($('[role="dialog"]')).not.toBeDisplayed();
    expect(existsSync(join(workspacesRoot, "test-agent-2"))).toBe(false);
    await expect($$('aside[aria-label="Agents"] li')).toBeElementsArrayOfSize(1);
  });

  it("shows Codex as unavailable in the provider select", async () => {
    await openDialog();
    await $("#agent-provider").click();
    const codex = $('[role="option"]*=Codex');
    await expect(codex).toHaveText(expect.stringContaining("Not installed"));
    await expect(codex).toHaveAttribute("aria-disabled", "true");
    await expect($('[role="option"]*=Claude Code')).not.toHaveAttribute("aria-disabled", "true");
    await browser.saveScreenshot(join(artifacts, "agents-provider-select.png"));
  });
});
