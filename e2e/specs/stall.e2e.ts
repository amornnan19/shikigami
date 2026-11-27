import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const messageBox = 'textarea[aria-label="Message"]';

describe("a provider that goes silent", () => {
  it("ends the turn with the stall message", async () => {
    await $("button=New agent").click();
    await $("#agent-name").setValue("Quiet Agent");
    await $("button=Create").click();
    await expect($(`${agentView} h2`)).toHaveText("Quiet Agent");
    await $(messageBox).setValue("STALL please");
    await browser.keys("Enter");
    const turn = $(`${agentView} article:nth-of-type(1)`);
    await expect(turn).toHaveText(
      expect.stringContaining("No response from the provider for 1 minute."),
      { wait: 30000 },
    );
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message", { wait: 10000 });
    await expect($('aside[aria-label="Agents"] [aria-label="Needs attention"]')).toBeExisting();
    await browser.saveScreenshot(join(artifacts, "recovery-stalled.png"));
  });

  it("answers the next message", async () => {
    await $(messageBox).setValue("Reply with the word pong");
    await browser.keys("Enter");
    const turn = $(`${agentView} article:nth-of-type(2)`);
    await expect(turn).toHaveText(expect.stringMatching(/word pong\s*pong$/), { wait: 20000 });
  });
});
