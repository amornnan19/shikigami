import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';

describe("after a restart", () => {
  it("shows the saved conversation of the agent", async () => {
    const row = $('aside[aria-label="Agents"] li:nth-child(1) button');
    await expect(row).toHaveText(expect.stringContaining("Test Agent"));
    await row.click();
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(6);
    const first = $(`${agentView} article:nth-of-type(1)`);
    await expect(first).toHaveText(expect.stringContaining("Reply with the word pong"));
    await expect(first).toHaveText(expect.stringContaining("pong"));
    const second = $(`${agentView} article:nth-of-type(2)`);
    await expect(second).toHaveText(expect.stringContaining("List the files in your folder"));
    await expect(second.$("button=2 steps")).toBeExisting();
    await expect(second).toHaveText(expect.stringContaining("eza"));
    await expect($(`${agentView} article:nth-of-type(3)`)).toHaveText(
      expect.stringContaining("Stopped"),
    );
    await expect($(`${agentView} article:nth-of-type(6)`)).toHaveText(
      expect.stringContaining("This reply was interrupted."),
    );
    await browser.saveScreenshot(join(artifacts, "chat-after-restart.png"));
  });

  it("keeps the queued message and starts nothing by itself", async () => {
    const queue = $('section[aria-label="Queue"]');
    await expect(queue).toHaveText(expect.stringContaining("Queued (1)"));
    await expect(queue).toHaveText(expect.stringContaining("queued before restart"));
    await expect($('button[aria-label="Stop"]')).not.toExist();
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(6);
    await browser.execute(() => {
      const scroller = document.querySelector('section[aria-label="Agent"] .overflow-y-auto');
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
    await expect($('[aria-label="Queue waiting"]')).toHaveText(
      expect.stringContaining(
        "The app closed while the agent was working. 1 queued message is waiting. Send a message or press Resume to continue.",
      ),
    );
    await expect($('[aria-label="Queue waiting"]').$("button=Resume")).toBeEnabled();
    await browser.saveScreenshot(join(artifacts, "chat-queue-after-restart.png"));
  });

  it("starts the queued message when Resume is pressed", async () => {
    await $('[aria-label="Queue waiting"]').$("button=Resume").click();
    const next = $(`${agentView} article:nth-of-type(7)`);
    await expect(next).toHaveText(expect.stringContaining("queued before restart"), {
      wait: 20000,
    });
    await expect(next).toHaveText(expect.stringMatching(/restart\s*pong$/), { wait: 20000 });
    await expect($('section[aria-label="Queue"]')).not.toExist();
    await expect($('[aria-label="Queue waiting"]')).not.toExist();
    await browser.execute(() => {
      const scroller = document.querySelector('section[aria-label="Agent"] .overflow-y-auto');
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
    await browser.saveScreenshot(join(artifacts, "chat-resumed.png"));
  });
});
