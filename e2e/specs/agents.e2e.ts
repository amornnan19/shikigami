import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { workspacesRoot } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");

async function openDialog() {
  await $("button=New agent").click();
  await expect($('[role="dialog"]')).toBeDisplayed();
}

describe("agents", () => {
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

  it("explains an invalid name under the field as soon as it is typed", async () => {
    await openDialog();
    await $("#agent-name").setValue("a".repeat(41));
    await expect($("#agent-name-error")).toHaveText("Use 1 to 40 characters.");
    await expect($("#agent-name")).toHaveAttribute("aria-invalid", "true");
    await expect($("button=Create")).toBeDisabled();
    await browser.saveScreenshot(join(artifacts, "agents-name-too-long.png"));
    await $("#agent-name").setValue("   ");
    await expect($("#agent-name-error")).toHaveText("The name cannot be empty.");
    await expect($("button=Create")).toBeDisabled();
    await browser.saveScreenshot(join(artifacts, "agents-name-empty.png"));
    await $("#agent-name").setValue("");
    await expect($("#agent-name-error")).not.toExist();
    await $("button=Cancel").click();
    await expect($('[role="dialog"]')).not.toBeDisplayed();
  });

  it("has no provider choice in the create dialog", async () => {
    await openDialog();
    await expect($("#agent-name")).toBeDisplayed();
    await expect($("#agent-provider")).not.toExist();
    await expect($("label=Provider")).not.toExist();
    await browser.saveScreenshot(join(artifacts, "agents-no-provider-choice.png"));
    await $("button=Cancel").click();
    await expect($('[role="dialog"]')).not.toBeDisplayed();
  });
});
