import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { fakeClaudeInvocations } from "../fake-providers";
import { revealLog, trashDir, workspacesRoot } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const agentView = 'section[aria-label="Agent"]';
const settings = '[role="dialog"]:has(#agent-settings-description)';
const confirm = '[role="dialog"]:has(#delete-agent-description)';
const instructions = `${settings} textarea[aria-label="Instructions"]`;
const messageBox = 'textarea[aria-label="Message"]';
const stopButton = 'button[aria-label="Stop"]';

async function createAgent(name: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $("button=Create").click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function selectAgent(name: string) {
  await $(sidebar).$(`button*=${name}`).click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function openSettings() {
  await $('button[aria-label="Agent settings"]').click();
}

async function closeSettings() {
  await browser.keys("Escape");
  await expect($(settings)).not.toExist();
}

async function scrollSettingsTo(section: string) {
  await browser.execute((label) => {
    document
      .querySelector(`[role="dialog"] section[aria-label="${label}"]`)
      ?.scrollIntoView({ block: "start" });
  }, section);
}

function lastInvocation() {
  return readFileSync(fakeClaudeInvocations, "utf8").split("\n").filter(Boolean).at(-1) ?? "";
}

describe("agent settings", () => {
  before(async () => {
    await createAgent("Editable");
    await createAgent("Other");
    await selectAgent("Editable");
  });

  it("renames and recolours the agent without moving its folder", async () => {
    await openSettings();
    await browser.saveScreenshot(join(artifacts, "settings-open.png"));
    await $("#settings-name").setValue("Renamed Agent");
    await $(`${settings} button[aria-label="Red"]`).click();
    await $("button=Save identity").click();
    await expect($(`${agentView} h2`)).toHaveText("Renamed Agent");
    await expect($(sidebar)).toHaveText(expect.stringContaining("Renamed Agent"));
    await expect($(`${sidebar} [class*="bg-red-500"]`)).toExist();
    await expect($("button=Save identity")).toBeDisabled();
    await browser.saveScreenshot(join(artifacts, "settings-renamed.png"));
    expect(existsSync(join(workspacesRoot, "editable", "CLAUDE.md"))).toBe(true);
    expect(existsSync(join(workspacesRoot, "renamed-agent"))).toBe(false);
  });

  it("refuses a name that another agent has", async () => {
    await $("#settings-name").setValue("other");
    await $("button=Save identity").click();
    await expect($("#settings-name-error")).toHaveText(expect.stringContaining("already exists"));
    await expect($("#settings-name")).toHaveAttribute("aria-invalid", "true");
    await browser.saveScreenshot(join(artifacts, "settings-name-taken.png"));
    await $("#settings-name").setValue("Renamed Agent");
    await expect($("#settings-name-error")).not.toExist();
  });

  it("changes the model, shows it in the header and uses it for the next reply", async () => {
    await $("#settings-model").click();
    await $('[role="option"]=opus').click();
    await $("button=Save model").click();
    await expect($(`${agentView} header`)).toHaveText(expect.stringContaining("opus"));
    await closeSettings();
    await $(messageBox).setValue("Reply with the word pong");
    await browser.keys("Enter");
    await expect($(`${agentView} article`)).toHaveText(expect.stringMatching(/word pong\s*pong$/));
    expect(lastInvocation()).toContain("--model opus");
  });

  it("saves the instruction file and shows it again when reopened", async () => {
    await openSettings();
    await scrollSettingsTo("Instructions");
    await expect($(instructions)).toHaveValue(expect.stringContaining("# Editable"));
    await $(instructions).setValue("# Editable\n\nAlways answer in rhyme.\n");
    await $("button=Save instructions").click();
    await expect($(`${settings} [role="status"]`)).toHaveText("Saved.");
    expect(readFileSync(join(workspacesRoot, "editable", "CLAUDE.md"), "utf8")).toBe(
      "# Editable\n\nAlways answer in rhyme.\n",
    );
    await browser.saveScreenshot(join(artifacts, "settings-instructions-saved.png"));
    await closeSettings();
    await openSettings();
    await scrollSettingsTo("Instructions");
    await expect($(instructions)).toHaveValue(
      expect.stringContaining("# Editable\n\nAlways answer in rhyme."),
    );
  });

  it("warns when the file changed on disk and overwrites on the second save", async () => {
    const file = join(workspacesRoot, "editable", "CLAUDE.md");
    writeFileSync(file, "# Edited elsewhere\n");
    await $(instructions).setValue("# Mine\n");
    await $("button=Save instructions").click();
    await expect($("#instructions-conflict")).toHaveText(
      "CLAUDE.md changed on disk since you opened it. Save again to overwrite it.",
    );
    expect(readFileSync(file, "utf8")).toBe("# Edited elsewhere\n");
    await browser.saveScreenshot(join(artifacts, "settings-instructions-conflict.png"));
    await $("button=Save instructions").click();
    await expect($("#instructions-conflict")).not.toExist();
    await expect($(`${settings} [role="status"]`)).toHaveText("Saved.");
    expect(readFileSync(file, "utf8")).toBe("# Mine\n");
  });

  it("shows the folder in Finder using the recorded path", async () => {
    await $("button=Show in Finder").click();
    await browser.waitUntil(() => existsSync(revealLog));
    expect(readFileSync(revealLog, "utf8").trim()).toBe(join(workspacesRoot, "editable"));
  });

  it("changes the sign-in choice of the agent", async () => {
    await scrollSettingsTo("Sign-in");
    await $("#settings-sign-in").click();
    await $('[role="option"]=API key').click();
    await $("button=Save sign-in").click();
    await closeSettings();
    await expect($(`${agentView} header`)).toHaveText(expect.stringContaining("API key"));
  });

  it("deletes without the checkbox and keeps the folder", async () => {
    await openSettings();
    await scrollSettingsTo("Delete");
    await $("button=Delete agent").click();
    await expect($(confirm)).toHaveText(expect.stringContaining("Delete Renamed Agent?"));
    await expect($(confirm)).toHaveText(
      expect.stringContaining("Its conversation is removed from Shikigami."),
    );
    await expect($(`${confirm} input[type="checkbox"]`)).not.toBeSelected();
    await browser.saveScreenshot(join(artifacts, "settings-delete-confirm.png"));
    await $(confirm).$("button=Delete").click();
    await expect($(sidebar)).not.toHaveText(expect.stringContaining("Renamed Agent"));
    await expect($$(`${sidebar} li`)).toBeElementsArrayOfSize(1);
    expect(existsSync(join(workspacesRoot, "editable", "CLAUDE.md"))).toBe(true);
    expect(existsSync(trashDir)).toBe(false);
    await browser.saveScreenshot(join(artifacts, "settings-deleted-folder-kept.png"));
  });

  it("deletes with the checkbox and moves the folder to the trash", async () => {
    await selectAgent("Other");
    await openSettings();
    await scrollSettingsTo("Delete");
    await $("button=Delete agent").click();
    await $(`${confirm} input[type="checkbox"]`).click();
    await expect($(confirm)).toHaveText(
      expect.stringContaining(`Also move the folder ${join(workspacesRoot, "other")} to the Trash`),
    );
    await browser.saveScreenshot(join(artifacts, "settings-delete-confirm-trash.png"));
    await $(confirm).$("button=Delete").click();
    await expect($$(`${sidebar} li`)).toBeElementsArrayOfSize(0);
    expect(existsSync(join(workspacesRoot, "other"))).toBe(false);
    expect(existsSync(join(trashDir, "other", "CLAUDE.md"))).toBe(true);
    expect(existsSync(join(workspacesRoot, "editable", "CLAUDE.md"))).toBe(true);
    await browser.saveScreenshot(join(artifacts, "settings-deleted-folder-trashed.png"));
  });

  it("refuses to delete a working agent until it is stopped", async () => {
    await createAgent("Busy");
    await $(messageBox).setValue("SLOW keep working");
    await browser.keys("Enter");
    await expect($(stopButton)).toBeEnabled({ wait: 15000 });
    await openSettings();
    await scrollSettingsTo("Delete");
    await expect($("button=Delete agent")).toBeDisabled();
    await expect($("#delete-hint")).toHaveText("Stop the agent before deleting it.");
    await browser.saveScreenshot(join(artifacts, "settings-delete-busy.png"));
    await closeSettings();
    await $(stopButton).click();
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message", { wait: 5000 });
    await openSettings();
    await scrollSettingsTo("Delete");
    await expect($("button=Delete agent")).toBeEnabled();
    await expect($("#delete-hint")).not.toExist();
    await $("button=Delete agent").click();
    await $(confirm).$("button=Delete").click();
    await expect($$(`${sidebar} li`)).toBeElementsArrayOfSize(0);
    expect(existsSync(join(workspacesRoot, "busy", "CLAUDE.md"))).toBe(true);
  });
});
