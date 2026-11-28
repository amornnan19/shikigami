import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { pickedFolder } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const groupList = 'section[aria-label="Group chats"]';
const agentView = 'section[aria-label="Agent"]';
const groupView = 'section[aria-label="Group"]';
const newGroup = '[role="dialog"]:has(#new-group-description)';
const messageBox = 'textarea[aria-label="Message"]';
const figures = `${agentView} header [data-figure]`;

async function createAgent(name: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $("button=Create").click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function figureCount(selector: string) {
  return (await $$(selector)).length;
}

describe("subagents as paper figures", () => {
  before(async () => {
    await createAgent("Ada");
    await createAgent("Bo");
    await $(sidebar).$("button*=Ada").click();
    await expect($(`${agentView} h2`)).toHaveText("Ada");
  });

  it("folds out one paper figure per running subagent beside the avatar", async () => {
    await $(messageBox).setValue("run subagents");
    await browser.keys("Enter");
    await browser.waitUntil(async () => (await figureCount(figures)) === 2, { timeout: 20000 });
    await browser.pause(700);
    await expect($(figures)).toHaveAttribute("data-phase", "working");
    await expect($(`${agentView} header [aria-label="Explore"]`)).toBeDisplayed();
    await expect($(`${agentView} header [aria-label="Plan"]`)).toBeDisplayed();
    await $(`${agentView} article`).$("button*=steps").click();
    await expect($(`${agentView} [data-subagent]`)).toBeDisplayed();
    expect(await figureCount(`${agentView} [data-subagent][data-status="running"]`)).toBe(2);
    await expect($(`${agentView} [data-subagent] [data-subagent-status]`)).toHaveText("Working");
    await browser.saveScreenshot(join(artifacts, "subagents-figures-out.png"));
  });

  it("folds the figures back when the subagents end and keeps nested groups in the turn", async () => {
    await expect($(`${agentView} article`)).toHaveText(
      expect.stringContaining("The scan is done, the outline failed."),
      { wait: 30000 },
    );
    await browser.waitUntil(async () => (await figureCount(figures)) === 0, { timeout: 5000 });
    const groups = await $$(`${agentView} [data-subagent]`);
    expect(groups).toHaveLength(2);
    const [explore, plan] = groups;
    await expect(explore).toHaveText(expect.stringContaining("Explore"));
    await expect(explore).toHaveText(expect.stringContaining("Scan the notes"));
    await expect(explore.$("[data-subagent-status]")).toHaveText("Done");
    await expect(plan).toHaveText(expect.stringContaining("Plan"));
    await expect(plan).toHaveText(expect.stringContaining("Draft the outline"));
    await expect(plan.$("[data-subagent-status]")).toHaveText("Failed");
    await expect(explore.$("button")).toHaveAttribute("aria-expanded", "false");
    await browser.saveScreenshot(join(artifacts, "subagents-nested-groups.png"));
    await explore.$("button").click();
    await expect(explore).toHaveText(expect.stringContaining("notes.txt"));
    await expect(explore).toHaveText(expect.stringContaining("Notes scanned."));
    await plan.$("button").click();
    await expect(plan).toHaveText(expect.stringContaining("ls -la"));
    await expect(plan).toHaveText(expect.stringContaining("Could not draft the outline."));
    await browser.saveScreenshot(join(artifacts, "subagents-failed.png"));
  });

  it("shows how many paper figures each member has out in the group header", async () => {
    await $("button=New group").click();
    await expect($(newGroup)).toBeDisplayed();
    await $("#group-name").setValue("Team");
    await $("button=Choose folder…").click();
    await expect($("#group-folder-path")).toHaveText(pickedFolder);
    await $(newGroup).$("label*=Ada").click();
    await $(newGroup).$("label*=Bo").click();
    await $("#group-lead").click();
    await $('[role="option"]=Ada').click();
    await $("button=Create").click();
    await expect($(`${groupView} h2`)).toHaveText("Team");
    await $(groupList).$("button*=Team").click();
    await $(messageBox).setValue("run subagents");
    await browser.keys("Enter");
    const badge = `${groupView} header [data-figure-count]`;
    await browser.waitUntil(async () => (await figureCount(badge)) === 1, { timeout: 20000 });
    await expect($(badge)).toHaveText("2");
    await expect($(badge)).toHaveAttribute("aria-label", "Ada: 2 paper figures out");
    await browser.pause(700);
    await expect($(`${groupView} article [data-figure]`)).toBeDisplayed();
    await browser.saveScreenshot(join(artifacts, "subagents-group-badge.png"));
    await expect($(`${groupView} article`)).toHaveText(
      expect.stringContaining("The scan is done, the outline failed."),
      { wait: 30000 },
    );
    await browser.waitUntil(async () => (await figureCount(badge)) === 0, { timeout: 5000 });
  });
});
