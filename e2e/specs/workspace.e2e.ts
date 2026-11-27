import { existsSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";
import { workspacesRoot } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const banner = '[aria-label="Agent problem"]';
const messageBox = 'textarea[aria-label="Message"]';
const folder = join(workspacesRoot, "test-agent");

async function refocus() {
  await browser.execute(() => {
    window.dispatchEvent(new Event("blur"));
    window.dispatchEvent(new Event("focus"));
  });
}

describe("a missing workspace folder", () => {
  before(async () => {
    await $('aside[aria-label="Agents"] li:nth-child(1) button').click();
    await expect($('section[aria-label="Agent"] h2')).toHaveText("Test Agent");
  });

  it("shows what is wrong and disables the composer", async () => {
    rmSync(folder, { recursive: true, force: true });
    await refocus();
    await expect($(banner)).toHaveText(
      expect.stringContaining("The workspace folder is missing."),
      { wait: 10000 },
    );
    await expect($(banner).$("button=Recreate folder")).toBeDisplayed();
    await expect($(messageBox)).toBeDisabled();
    await expect($(messageBox)).toHaveAttribute("placeholder", "The workspace folder is missing.");
    await browser.saveScreenshot(join(artifacts, "recovery-workspace-missing.png"));
  });

  it("recreates the folder and enables the composer again", async () => {
    await $(banner).$("button=Recreate folder").click();
    await expect($(banner)).not.toExist({ wait: 10000 });
    await expect($(messageBox)).toBeEnabled();
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message");
    expect(existsSync(join(folder, "CLAUDE.md"))).toBe(true);
    expect(readFileSync(join(folder, "CLAUDE.md"), "utf8")).toContain("# Test Agent");
    await browser.saveScreenshot(join(artifacts, "recovery-workspace-recreated.png"));
  });
});
