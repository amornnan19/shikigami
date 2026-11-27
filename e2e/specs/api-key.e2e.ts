import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const claudeRow = 'section[aria-label="Providers"] li:nth-child(1)';
const apiKeyBlock = `${claudeRow} section[aria-label="API key"]`;
const fakeKey = "sk-ant-test-00000000wxyz";

describe("api key", () => {
  it("starts with Add API key", async () => {
    await expect($(apiKeyBlock)).toBeExisting();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("Add API key"));
    await expect($(claudeRow)).not.toHaveText(expect.stringContaining("API key saved"));
  });

  it("refuses a key with the wrong shape under the field", async () => {
    await $("button=Add API key").click();
    await expect($("#api-key-input")).toHaveAttribute("type", "password");
    await $("#api-key-input").setValue("not-an-anthropic-key");
    await $("button=Save").click();
    await expect($("#api-key-error")).toHaveText(expect.stringContaining("sk-ant-"));
    await expect($(apiKeyBlock)).not.toHaveText(expect.stringContaining("API key saved"));
    await browser.saveScreenshot(join(artifacts, "api-key-invalid.png"));
    await $("button=Cancel").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("Add API key"));
  });

  it("saves a key and shows only the last four characters", async () => {
    await $("button=Add API key").click();
    await $("#api-key-input").setValue(fakeKey);
    await $("button=Save").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("API key saved (…wxyz)"));
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready (API key)"));
    await expect($("#api-key-enabled")).toHaveAttribute("aria-checked", "true");
    await expect($("#api-key-input")).not.toBeExisting();
    const page = await $("body").getText();
    expect(page).not.toContain(fakeKey);
    await browser.saveScreenshot(join(artifacts, "api-key-saved.png"));
  });

  it("falls back to the CLI sign-in when the switch is off", async () => {
    await $("#api-key-enabled").click();
    await expect($("#api-key-enabled")).toHaveAttribute("aria-checked", "false");
    await expect($(claudeRow)).not.toHaveText(expect.stringContaining("Ready (API key)"));
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready"));
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("API key saved (…wxyz)"));
    await browser.saveScreenshot(join(artifacts, "api-key-off.png"));
  });

  it("uses the key again when the switch is on", async () => {
    await $("#api-key-enabled").click();
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready (API key)"));
  });

  it("removes the key after a confirmation", async () => {
    await $(apiKeyBlock).$("button=Remove").click();
    await expect($('[role="dialog"]')).toHaveText(
      expect.stringContaining("Claude Code agents will use the CLI sign-in."),
    );
    await browser.saveScreenshot(join(artifacts, "api-key-remove-confirm.png"));
    await $('[role="dialog"]').$("button=Remove").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("Add API key"));
    await expect($(claudeRow)).not.toHaveText(expect.stringContaining("Ready (API key)"));
    await expect($(claudeRow)).toHaveText(expect.stringContaining("Ready"));
    await browser.saveScreenshot(join(artifacts, "api-key-removed.png"));
  });
});
