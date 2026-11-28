import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const agentView = 'section[aria-label="Agent"]';
const settings = '[role="dialog"]:has(#agent-settings-description)';
const slow = 25000;

function row(name: string) {
  return $(`${settings} li[data-routine="${name}"]`);
}

describe("routines after the app was closed", () => {
  before(async () => {
    await $(sidebar).$("button*=Runner").click();
    await expect($(`${agentView} h2`)).toHaveText("Runner");
  });

  it("runs a once routine missed by hours as late and shows Done", async () => {
    await $('button[aria-label="Agent settings"]').click();
    await browser.execute(() => {
      document
        .querySelector('[role="dialog"] section[aria-label="Routines"]')
        ?.scrollIntoView({ block: "start" });
    });
    await expect(row("Soon once").$('[data-part="status"]')).toHaveText("Done", { wait: slow });
    await expect(row("Soon once").$('[data-part="last-run"]')).toHaveText(
      "Last run: Ran late 12:00",
      { wait: slow },
    );
  });

  it("records a once routine missed by more than a day as Missed", async () => {
    await expect(row("Old once").$('[data-part="status"]')).toHaveText("Done");
    await expect(row("Old once").$('[data-part="last-run"]')).toHaveText("Last run: Missed");
  });

  it("records one skip for a repeating routine and keeps its next time", async () => {
    await expect(row("Morning").$('[data-part="last-run"]')).toHaveText(
      "Last run: Skipped: Shikigami was not running.",
    );
    await expect(row("Morning").$('[data-part="status"]')).toHaveText(
      "Next: Tue 1 Dec 2026, 08:00",
    );
    await expect(row("Slow check").$('[data-part="status"]')).toHaveText(
      "Next: Thu 3 Dec 2099, 08:00",
    );
    await row("Old once").scrollIntoView({ block: "center" });
    await browser.saveScreenshot(join(artifacts, "routine-runs-late-missed.png"));
    await row("Morning").$("button=History").click();
    await expect(row("Morning").$('[data-part="history"]')).toHaveText(
      expect.stringContaining("Skipped: Shikigami was not running."),
    );
    await row("Morning").scrollIntoView({ block: "center" });
    await browser.saveScreenshot(join(artifacts, "routine-runs-skipped-history.png"));
  });

  it("shows the late run in the chat with its marker", async () => {
    await browser.keys("Escape");
    await expect($(settings)).not.toExist();
    await expect($(agentView)).toHaveText(expect.stringContaining("Routine: Soon once"), {
      wait: slow,
    });
    await expect($$(`${agentView} [data-part="routine-marker"]`)).toBeElementsArrayOfSize(6);
    await browser.execute(() => {
      const scroller = document.querySelector('section[aria-label="Agent"] .overflow-y-auto');
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
    await browser.saveScreenshot(join(artifacts, "routine-runs-late-chat.png"));
  });
});
