import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const messageBox = 'textarea[aria-label="Message"]';

describe("subagents under reduced motion", () => {
  it("shows a static stacked badge with the count instead of moving figures", async () => {
    await $("button=New agent").click();
    await $("#agent-name").setValue("Ada");
    await $("button=Create").click();
    await expect($(`${agentView} h2`)).toHaveText("Ada");
    await $(messageBox).setValue("run subagents");
    await browser.keys("Enter");
    const badge = `${agentView} header [data-figure-badge]`;
    await expect($(badge)).toBeDisplayed({ wait: 20000 });
    await expect($(badge)).toHaveAttribute("aria-label", "2 paper figures out");
    await expect($(badge)).toHaveText("2");
    expect(await $$(`${agentView} [data-figure]`)).toHaveLength(0);
    expect(await $$(`${agentView} .paper-figure`)).toHaveLength(0);
    await browser.pause(700);
    await browser.saveScreenshot(join(artifacts, "subagents-reduced-motion.png"));
    await expect($(`${agentView} article`)).toHaveText(
      expect.stringContaining("The scan is done, the outline failed."),
      { wait: 30000 },
    );
    await expect($(badge)).not.toBeDisplayed();
  });
});
