import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const messageBox = 'textarea[aria-label="Message"]';
const stopButton = 'button[aria-label="Stop"]';
const queue = 'section[aria-label="Queue"]';
const waiting = '[aria-label="Queue waiting"]';

describe("a sign-in failure after the provider started", () => {
  it("holds the queued message and shows the sign-in guidance", async () => {
    await $("button=New agent").click();
    await $("#agent-name").setValue("Signed Out Agent");
    await $("button=Create").click();
    await expect($(`${agentView} h2`)).toHaveText("Signed Out Agent");
    await $(messageBox).setValue("SIGNIN please");
    await browser.keys("Enter");
    await expect($(stopButton)).toBeDisplayed();
    await $(messageBox).setValue("queued after sign-in");
    await browser.keys("Enter");
    await expect($(queue)).toHaveText(expect.stringContaining("queued after sign-in"), {
      wait: 15000,
    });
    const turn = $(`${agentView} article:nth-of-type(1)`);
    await expect(turn).toHaveText(
      expect.stringContaining(
        "Invalid API key · Please run /login Sign in again, then press Resume.",
      ),
      { wait: 30000 },
    );
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message", { wait: 10000 });
    await expect($(waiting)).toHaveText(expect.stringContaining("1 queued message is waiting."));
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(1);
    await browser.saveScreenshot(join(artifacts, "sign-in-failed-held.png"));
  });

  it("runs the queued message when Resume is pressed", async () => {
    await $(waiting).$("button=Resume").click();
    const next = $(`${agentView} article:nth-of-type(2)`);
    await expect(next).toHaveText(expect.stringContaining("queued after sign-in"), {
      wait: 20000,
    });
    await expect(next).toHaveText(expect.stringMatching(/sign-in\s*pong$/), { wait: 20000 });
    await expect($(waiting)).not.toExist();
    await browser.saveScreenshot(join(artifacts, "sign-in-resumed.png"));
  });
});
