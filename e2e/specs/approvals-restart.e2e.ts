import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const groupList = 'section[aria-label="Group chats"]';
const agentView = 'section[aria-label="Agent"]';
const groupView = 'section[aria-label="Group"]';
const card = 'section[aria-label="Approval request"]';

describe("approval cards after a restart", () => {
  it("keeps the outcome and the time of every past card in an agent chat", async () => {
    await $(sidebar).$("button*=Ada").click();
    await expect($(`${agentView} h2`)).toHaveText("Ada");
    await expect($$(`${agentView} ${card}`)).toBeElementsArrayOfSize(7, { wait: 20000 });
    const outcomes = await $$(`${agentView} ${card} [data-approval-outcome]`).map((outcome) =>
      outcome.getText(),
    );
    expect(outcomes.map((text) => text.replace(/\s*\d{1,2}:\d{2}\s?[AP]M$/, ""))).toEqual([
      "Allowed",
      "Denied",
      "Denied",
      "Allowed",
      "Timed out",
      "Denied because the turn stopped",
      "Denied because Shikigami closed",
    ]);
    for (const text of outcomes) {
      expect(text).toMatch(/\d{1,2}:\d{2}\s?[AP]M$/);
    }
    await expect($(`${agentView} ${card} button`)).not.toExist();
    await expect($(`${sidebar} [aria-label="Waiting for you"]`)).not.toExist();
    await browser.execute(() => {
      const scroller = document.querySelector('section[aria-label="Agent"] .overflow-y-auto');
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
    await browser.saveScreenshot(join(artifacts, "approvals-after-restart.png"));
  });

  it("keeps the card of a group chat", async () => {
    await $(groupList).$("button*=Team").click();
    await expect($(`${groupView} h2`)).toHaveText("Team");
    await expect($$(`${groupView} ${card}`)).toBeElementsArrayOfSize(1, { wait: 20000 });
    await expect($(`${groupView} ${card}`)).toHaveText(expect.stringContaining("Allowed"));
    await expect($(`${groupList} [aria-label="Waiting for you"]`)).not.toExist();
    await browser.saveScreenshot(join(artifacts, "approvals-group-after-restart.png"));
  });
});
