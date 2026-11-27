import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const claudeCard = 'section[aria-label="Claude"]';
const apiKeyBlock = `${claudeCard} section[aria-label="API key"]`;
const agentHeader = 'section[aria-label="Agent"] header';
const fakeKey = "sk-ant-test-00000000wxyz";

async function showClaudeSection() {
  await browser.refresh();
  await $(claudeCard).waitForExist();
  await expect($(`${claudeCard} #auth-mode-subscription`)).toBeExisting();
}

async function createAgent(name: string, signIn: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $("#agent-sign-in").click();
  await $(`[role="option"]=${signIn}`).click();
  await $("button=Create").click();
  await expect($('section[aria-label="Agent"] h2')).toHaveText(name);
}

async function selectAgent(name: string) {
  await $('aside[aria-label="Agents"]').$(`button*=${name}`).click();
  await expect($('section[aria-label="Agent"] h2')).toHaveText(name);
}

describe("sign-in choice", () => {
  it("makes API key the app default by choosing it and adding a key", async () => {
    await showClaudeSection();
    await $("#auth-mode-api-key").click();
    await $("#api-key-input").setValue(fakeKey);
    await $("button=Save").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("API key saved (…wxyz)"));
    await expect($("#auth-mode-api-key")).toHaveAttribute("aria-checked", "true");
    await browser.saveScreenshot(join(artifacts, "auth-mode-app-api-key.png"));
  });

  it("offers the app default in the create dialog and shows the mode in the chat header", async () => {
    await $("button=New agent").click();
    await $("#agent-sign-in").click();
    await expect($('[role="option"]=Use app default (API key)')).toBeDisplayed();
    await expect($('[role="option"]=Claude subscription')).toBeDisplayed();
    await expect($('[role="option"]=API key')).toBeDisplayed();
    await browser.saveScreenshot(join(artifacts, "auth-mode-dialog-options.png"));
    await $('[role="option"]=Use app default (API key)').click();
    await $("button=Cancel").click();
    await createAgent("Follower", "Use app default (API key)");
    await expect($(agentHeader)).toHaveText(expect.stringContaining("API key"));
    await browser.saveScreenshot(join(artifacts, "auth-mode-follower-api-key.png"));
  });

  it("creates an agent with an explicit API key choice", async () => {
    await createAgent("Pinned", "API key");
    await expect($(agentHeader)).toHaveText(expect.stringContaining("API key"));
  });

  it("changes the follower but not the pinned agent when the app switches to subscription", async () => {
    await showClaudeSection();
    await $("#auth-mode-subscription").click();
    await expect($("#auth-mode-subscription")).toHaveAttribute("aria-checked", "true");
    await selectAgent("Follower");
    await expect($(agentHeader)).toHaveText(expect.stringContaining("Subscription"));
    await browser.saveScreenshot(join(artifacts, "auth-mode-follower-subscription.png"));
    await selectAgent("Pinned");
    await expect($(agentHeader)).toHaveText(expect.stringContaining("API key"));
    await browser.saveScreenshot(join(artifacts, "auth-mode-pinned-api-key.png"));
  });

  it("fails a turn at once with no saved key in API key mode", async () => {
    await showClaudeSection();
    await $(apiKeyBlock).$("button=Remove").click();
    await $('[role="dialog"]').$("button=Remove").click();
    await expect($(apiKeyBlock)).toHaveText(expect.stringContaining("Add API key"));
    await selectAgent("Pinned");
    await $('textarea[aria-label="Message"]').setValue("hello");
    await browser.keys("Enter");
    await expect($('section[aria-label="Agent"]')).toHaveText(
      expect.stringContaining(
        "No API key is saved. Add one or switch this agent to your Claude subscription.",
      ),
    );
    await browser.saveScreenshot(join(artifacts, "auth-mode-no-key.png"));
  });
});
