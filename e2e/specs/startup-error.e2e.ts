import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";

describe("startup error", () => {
  it("shows the error screen when the database file is broken", async () => {
    await expect($("h1")).toHaveText("Shikigami could not start");
    await expect($("[role=alert]")).toHaveText(expect.stringContaining("could not"));
    await expect($("main")).toHaveText(expect.stringContaining("Data folder:"));
    await browser.saveScreenshot(join(import.meta.dirname, "..", "artifacts", "startup-error.png"));
  });
});
