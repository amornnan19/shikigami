import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const claudeCard = 'section[aria-label="Claude"]';
const apiKeyBlock = `${claudeCard} section[aria-label="API key"]`;
const fakeKey = "sk-ant-test-00000000wxyz";

describe("api key", () => {
  it("starts with Add API key", async () => {
    await expect($(apiKeyBlock)).toBeExisting();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("Add API key"));
    await expect($(claudeCard)).not.toHaveText(expect.stringContaining("API key saved"));
  });

  it("opens the add form when API key is chosen with no key saved, without saving the choice yet", async () => {
    await $("#auth-mode-api-key").click();
    await expect($("#auth-mode-api-key")).toHaveAttribute("aria-checked", "true");
    await expect($("#api-key-input")).toBeDisplayed();
    await expect($("#api-key-input")).toHaveAttribute("type", "password");
    await browser.saveScreenshot(join(artifacts, "api-key-form-opened.png"));
  });

  it("refuses a key with the wrong shape under the field", async () => {
    await $("#api-key-input").setValue("not-an-anthropic-key");
    await $("button=Save").click();
    await expect($("#api-key-error")).toHaveText(expect.stringContaining("sk-ant-"));
    await expect($(apiKeyBlock)).not.toHaveText(expect.stringContaining("API key saved"));
    await browser.saveScreenshot(join(artifacts, "api-key-invalid.png"));
  });

  it("leaves the choice unchanged when the form is cancelled", async () => {
    await $("button=Cancel").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("Add API key"));
    await expect($("#auth-mode-subscription")).toHaveAttribute("aria-checked", "true");
    await expect($("#auth-mode-api-key")).toHaveAttribute("aria-checked", "false");
    await browser.refresh();
    await $("#auth-mode-subscription").waitForExist();
    await expect($("#auth-mode-subscription")).toHaveAttribute("aria-checked", "true");
    await browser.saveScreenshot(join(artifacts, "api-key-cancelled.png"));
  });

  it("saves a key, chooses API key, and shows only the last four characters", async () => {
    await $("#auth-mode-api-key").click();
    await $("#api-key-input").setValue(fakeKey);
    await $("button=Save").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("API key saved (…wxyz)"));
    await expect($("#api-key-input")).not.toBeExisting();
    await expect($("#auth-mode-api-key")).toHaveAttribute("aria-checked", "true");
    const page = await $("body").getText();
    expect(page).not.toContain(fakeKey);
    await browser.saveScreenshot(join(artifacts, "api-key-saved.png"));
  });

  it("keeps the saved key when the subscription is chosen", async () => {
    await $("#auth-mode-subscription").click();
    await expect($("#auth-mode-subscription")).toHaveAttribute("aria-checked", "true");
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("API key saved (…wxyz)"));
    await browser.saveScreenshot(join(artifacts, "api-key-subscription-chosen.png"));
  });

  it("removes the key after a confirmation", async () => {
    await $(apiKeyBlock).$("button=Remove").click();
    await expect($('[role="dialog"]')).toHaveText(
      expect.stringContaining("switch them to your Claude subscription"),
    );
    await browser.saveScreenshot(join(artifacts, "api-key-remove-confirm.png"));
    await $('[role="dialog"]').$("button=Remove").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("Add API key"));
    await expect($("#auth-mode-subscription")).toHaveAttribute("aria-checked", "true");
    await browser.saveScreenshot(join(artifacts, "api-key-removed.png"));
  });
});
