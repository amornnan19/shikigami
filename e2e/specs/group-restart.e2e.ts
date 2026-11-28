import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const groupList = 'section[aria-label="Group chats"]';
const groupView = 'section[aria-label="Group"]';
const queue = 'section[aria-label="Queue"]';
const waiting = '[aria-label="Queue waiting"]';

async function scrollToEnd() {
  await browser.execute(() => {
    const scroller = document.querySelector('section[aria-label="Group"] .overflow-y-auto');
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });
}

describe("a group chat after a restart", () => {
  it("shows the saved transcript with its authors and the interrupted turn", async () => {
    await $(groupList).$("button*=Team").click();
    await expect($(`${groupView} h2`)).toHaveText("Team");
    await expect($$(`${groupView} article`)).toBeElementsArrayOfSize(7);
    const first = $(`${groupView} article:nth-of-type(1)`);
    await expect(first).toHaveText(expect.stringContaining("hello"));
    await expect(first).toHaveText(expect.stringMatching(/Ada\s*pong$/));
    await expect($(`${groupView} article:nth-of-type(3)`)).toHaveText(
      expect.stringContaining("Stopped"),
    );
    await expect($(`${groupView} article:nth-of-type(7)`)).toHaveText(
      expect.stringContaining("This reply was interrupted."),
    );
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "group-restart-transcript.png"));
  });

  it("keeps the queued message and starts nothing by itself", async () => {
    await expect($(queue)).toHaveText(expect.stringContaining("Queued (1)"));
    await expect($(queue)).toHaveText(expect.stringContaining("queued before restart"));
    await expect($('button[aria-label="Stop"]')).not.toExist();
    await expect($$(`${groupView} article`)).toBeElementsArrayOfSize(7);
    await scrollToEnd();
    await expect($(waiting)).toHaveText(
      expect.stringContaining(
        "The app closed while the agent was working. 1 queued message is waiting. Send a message or press Resume to continue.",
      ),
    );
    await expect($(waiting).$("button=Resume")).toBeEnabled();
    await browser.saveScreenshot(join(artifacts, "group-restart-queue.png"));
  });

  it("starts the queued message when Resume is pressed", async () => {
    await $(waiting).$("button=Resume").click();
    const next = $(`${groupView} article:nth-of-type(8)`);
    await expect(next).toHaveText(expect.stringContaining("queued before restart"), {
      wait: 20000,
    });
    await expect(next).toHaveText(expect.stringMatching(/Ada\s*pong$/), { wait: 20000 });
    await expect($(queue)).not.toExist();
    await expect($(waiting)).not.toExist();
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "group-restart-resumed.png"));
  });
});
