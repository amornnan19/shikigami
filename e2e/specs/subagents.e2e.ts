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

async function createAgent(name: string, color: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $(`[role="radio"][aria-label="${color}"]`).click();
  await $("button=Create").click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function scrollToTop(view: string) {
  await browser.execute((selector: string) => {
    const scroller = document.querySelector(`${selector} .overflow-y-auto`);
    if (scroller) scroller.scrollTop = 0;
  }, view);
}

async function figureCount(selector: string) {
  return (await $$(selector)).length;
}

describe("subagents as paper figures", () => {
  before(async () => {
    await createAgent("Ada", "Violet");
    await createAgent("Bo", "Teal");
    await $(sidebar).$("button*=Ada").click();
    await expect($(`${agentView} h2`)).toHaveText("Ada");
  });

  it("folds out one paper figure per running subagent beside the avatar", async () => {
    await browser.execute(() => {
      const seen: string[] = [];
      (window as unknown as { __phases: string[] }).__phases = seen;
      new MutationObserver((records) => {
        for (const record of records) {
          const nodes = [record.target, ...Array.from(record.addedNodes)];
          for (const node of nodes) {
            if (!(node instanceof Element)) continue;
            for (const figure of [node, ...Array.from(node.querySelectorAll("[data-figure]"))]) {
              const id = figure.getAttribute("data-figure");
              const phase = figure.getAttribute("data-phase");
              if (id && phase) seen.push(`${figure.getAttribute("aria-label")}:${phase}`);
            }
          }
        }
      }).observe(document.body, {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ["data-phase"],
      });
    });
    await $(messageBox).setValue("run subagents");
    await browser.keys("Enter");
    await browser.waitUntil(async () => (await figureCount(figures)) === 2, { timeout: 20000 });
    await browser.pause(700);
    await expect($(figures)).toHaveAttribute("data-phase", "working");
    await expect($(`${agentView} header [aria-label="Explore"]`)).toBeDisplayed();
    await expect($(`${agentView} header [aria-label="Plan"]`)).toBeDisplayed();
    await expect($(`${agentView} article`).$("button*=2 subagents")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    await expect($(`${agentView} [data-subagent]`)).toBeDisplayed();
    expect(await figureCount(`${agentView} [data-subagent][data-status="running"]`)).toBe(2);
    await expect($(`${agentView} [data-subagent] [data-subagent-status]`)).toHaveText("Working");
    await scrollToTop(agentView);
    await browser.saveScreenshot(join(artifacts, "subagents-figures-out.png"));
  });

  it("folds the figures back when the subagents end and keeps nested groups in the turn", async () => {
    await expect($(`${agentView} article`)).toHaveText(
      expect.stringContaining("The scan is done, the outline failed."),
      { wait: 30000 },
    );
    await browser.waitUntil(async () => (await figureCount(figures)) === 0, { timeout: 5000 });
    const phases = await browser.execute(
      () => (window as unknown as { __phases: string[] }).__phases,
    );
    expect(phases).toContain("Explore:working");
    expect(phases).toContain("Explore:folding");
    expect(phases).toContain("Plan:dissolving");
    expect(phases.indexOf("Explore:working")).toBeLessThan(phases.indexOf("Explore:folding"));
    expect(phases.indexOf("Plan:working")).toBeLessThan(phases.indexOf("Plan:dissolving"));
    await expect($(`${agentView} article`).$("button*=2 subagents")).toHaveText(
      "2 subagents · 2 steps",
    );
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
    await expect(explore.$("[data-subagent-task]")).toHaveText(
      expect.stringContaining("Read notes.txt and summarise it."),
    );
    await expect(explore.$("[data-subagent-result]")).toHaveText(
      expect.stringContaining("Notes scanned."),
    );
    await expect(explore).toHaveText(expect.stringContaining("notes.txt"));
    await expect(explore).not.toHaveText(expect.stringContaining("subagent_type"));
    await expect(explore.$("button=Show more")).toBeDisplayed();
    await browser.saveScreenshot(join(artifacts, "subagents-group-open.png"));
    await explore.$("button=Show more").click();
    await expect(explore.$("button=Show less")).toBeDisplayed();
    await explore.$("button=Show less").click();
    await plan.$("button").click();
    await expect(plan.$("[data-subagent-task]")).toHaveText(
      expect.stringContaining("Plan the outline."),
    );
    await expect(plan).toHaveText(expect.stringContaining("ls -la"));
    await expect(plan.$("[data-subagent-result]")).toHaveText(
      expect.stringContaining("Could not draft the outline."),
    );
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
    await scrollToTop(groupView);
    await browser.saveScreenshot(join(artifacts, "subagents-group-badge.png"));
    await expect($(`${groupView} article`)).toHaveText(
      expect.stringContaining("The scan is done, the outline failed."),
      { wait: 30000 },
    );
    await browser.waitUntil(async () => (await figureCount(badge)) === 0, { timeout: 5000 });
  });
});
