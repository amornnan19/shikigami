import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";

describe("launch", () => {
  it("opens the main window with the placeholder text", async () => {
    await expect(browser).toHaveTitle("Shikigami");
    await expect($("h1")).toHaveText("Shikigami");
    await expect($("p")).toHaveText("Coding agents on your Mac.");
    await browser.saveScreenshot(join(import.meta.dirname, "..", "artifacts", "launch.png"));
  });
});
