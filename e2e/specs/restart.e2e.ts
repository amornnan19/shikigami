import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';

describe("after a restart", () => {
  it("shows the saved conversation of the agent", async () => {
    const row = $('aside[aria-label="Agents"] li:nth-child(1) button');
    await expect(row).toHaveText(expect.stringContaining("Test Agent"));
    await row.click();
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(2);
    const first = $(`${agentView} article:nth-of-type(1)`);
    await expect(first).toHaveText(expect.stringContaining("Reply with the word pong"));
    await expect(first).toHaveText(expect.stringContaining("pong"));
    const second = $(`${agentView} article:nth-of-type(2)`);
    await expect(second).toHaveText(expect.stringContaining("List the files in your folder"));
    await expect(second.$("button=2 steps")).toBeExisting();
    await expect(second).toHaveText(expect.stringContaining("eza"));
    await browser.saveScreenshot(join(artifacts, "chat-after-restart.png"));
  });
});
