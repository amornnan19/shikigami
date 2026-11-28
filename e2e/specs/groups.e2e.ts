import { existsSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { groupsRoot, pickedFolder } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const groupList = 'section[aria-label="Group chats"]';
const agentView = 'section[aria-label="Agent"]';
const groupView = 'section[aria-label="Group"]';
const newGroup = '[role="dialog"]:has(#new-group-description)';
const settings = '[role="dialog"]:has(#group-settings-description)';
const confirm = '[role="dialog"]:has(#delete-group-description)';
const agentConfirm = '[role="dialog"]:has(#delete-agent-description)';

async function createAgent(name: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $("button=Create").click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function chooseMembers(...names: string[]) {
  for (const name of names) {
    await $(newGroup).$(`label*=${name}`).click();
  }
}

async function createGroup(name: string, lead: string, ...members: string[]) {
  await $("button=New group").click();
  await expect($(newGroup)).toBeDisplayed();
  await $("#group-name").setValue(name);
  await $("button=Choose folder…").click();
  await expect($("#group-folder-path")).toHaveText(pickedFolder);
  await chooseMembers(...members);
  await $("#group-lead").click();
  await $(`[role="option"]=${lead}`).click();
  await $("button=Create").click();
  await expect($(`${groupView} h2`)).toHaveText(name);
}

async function openSettings() {
  await $('button[aria-label="Group settings"]').click();
  await expect($(settings)).toBeDisplayed();
}

describe("group chats", () => {
  before(async () => {
    await createAgent("Ada");
    await createAgent("Bo");
  });

  it("shows the sidebar sections and opens the dialog with a suggested folder", async () => {
    await expect($(sidebar).$("h2=Agents")).toBeDisplayed();
    await expect($(`${groupList} h2`)).toHaveText("Group chats");
    await expect($(groupList)).toHaveText(expect.stringContaining("No group chats yet."));
    await $("button=New group").click();
    await expect($(newGroup)).toBeDisplayed();
    await expect($("button=Create")).toBeDisabled();
    await $("#group-name").setValue("Team");
    await expect($("#group-folder-path")).toHaveText(join(groupsRoot, "team"));
    await expect($("button=Create")).toBeDisabled();
    await chooseMembers("Ada");
    await expect($("button=Create")).toBeDisabled();
    await browser.saveScreenshot(join(artifacts, "groups-dialog.png"));
    await $("button=Cancel").click();
    await expect($(newGroup)).not.toExist();
    expect(existsSync(join(groupsRoot, "team"))).toBe(false);
  });

  it("creates a group with a picked folder and lists it", async () => {
    await createGroup("Team", "Bo", "Ada", "Bo");
    await expect($(`${groupList} li:nth-child(1) button`)).toHaveText(
      expect.stringContaining("Team"),
    );
    await expect($(`${groupList} li:nth-child(1) button`)).toHaveAttribute("aria-current", "true");
    await expect($("#group-header-folder")).toHaveText(pickedFolder);
    await expect($(groupView)).toHaveText(expect.stringContaining("No messages yet."));
    await expect($(`${groupView} [role="alert"]`)).not.toExist();
    await browser.saveScreenshot(join(artifacts, "groups-created.png"));
    expect(existsSync(join(groupsRoot, "team"))).toBe(false);
  });

  it("refuses a duplicate name under the name field", async () => {
    await $("button=New group").click();
    await $("#group-name").setValue("team");
    await chooseMembers("Ada", "Bo");
    await $("button=Create").click();
    await expect($("#group-name-error")).toHaveText(expect.stringContaining("already exists"));
    await browser.saveScreenshot(join(artifacts, "groups-name-taken.png"));
    await $("button=Cancel").click();
    await expect($(newGroup)).not.toExist();
  });

  it("renames the group", async () => {
    await openSettings();
    await browser.saveScreenshot(join(artifacts, "groups-settings.png"));
    await $("#group-settings-name").setValue("Crew");
    await $("button=Save name").click();
    await expect($(`${groupView} h2`)).toHaveText("Crew");
    await expect($(groupList)).toHaveText(expect.stringContaining("Crew"));
    await expect($("button=Save name")).toBeDisabled();
    await browser.keys("Escape");
    await expect($(settings)).not.toExist();
  });

  it("warns when another group uses the folder, then deletes that group and keeps the folder", async () => {
    await createGroup("Second", "Ada", "Ada", "Bo");
    await expect($(`${groupView} [role="status"]`)).toHaveText("Another group uses this folder.");
    await browser.saveScreenshot(join(artifacts, "groups-shared-folder.png"));
    await openSettings();
    await $("button=Delete group").click();
    await expect($(confirm)).toHaveText(expect.stringContaining("Delete Second?"));
    await expect($(confirm)).toHaveText(
      expect.stringContaining("Its messages are removed from Shikigami. The folder is kept."),
    );
    await browser.saveScreenshot(join(artifacts, "groups-delete-confirm.png"));
    await $(confirm).$("button=Delete").click();
    await expect($$(`${groupList} li`)).toBeElementsArrayOfSize(1);
    await expect($(groupList)).not.toHaveText(expect.stringContaining("Second"));
    expect(existsSync(join(pickedFolder, "keep.txt"))).toBe(true);
    await browser.saveScreenshot(join(artifacts, "groups-deleted-folder-kept.png"));
  });

  it("shows a group with one member left after its agent is deleted", async () => {
    await $(sidebar).$("button*=Bo").click();
    await expect($(`${agentView} h2`)).toHaveText("Bo");
    await $('button[aria-label="Agent settings"]').click();
    await $("button=Delete agent").click();
    await $(agentConfirm).$("button=Delete").click();
    await expect($(`${sidebar} ul li`)).toHaveText(expect.stringContaining("Ada"));
    await $(groupList).$("button*=Crew").click();
    await expect($(`${groupView} [role="alert"]`)).toHaveText(
      "Add another agent to use this group.",
    );
    await browser.saveScreenshot(join(artifacts, "groups-too-few-members.png"));
    await openSettings();
    await expect($("#group-settings-lead")).toHaveText("Ada");
    await expect($("button=Save members")).toBeDisabled();
    await browser.saveScreenshot(join(artifacts, "groups-lead-reassigned.png"));
    await browser.keys("Escape");
    expect(existsSync(join(pickedFolder, "keep.txt"))).toBe(true);
  });
});
