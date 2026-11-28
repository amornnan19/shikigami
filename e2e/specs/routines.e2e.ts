import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const agentView = 'section[aria-label="Agent"]';
const settings = '[role="dialog"]:has(#agent-settings-description)';
const editor = '[role="dialog"]:has(#routine-editor-description)';
const confirm = '[role="dialog"]:has(#delete-routine-description)';
const nextText = /^Next: [A-Z][a-z]{2} \d{1,2} [A-Z][a-z]{2} \d{4}, \d{2}:\d{2}$/;

async function setNative(selector: string, value: string) {
  await browser.execute(
    (target, text) => {
      const input = document.querySelector(target) as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    },
    selector,
    value,
  );
}

async function scrollToRoutines() {
  await browser.execute(() => {
    document
      .querySelector('[role="dialog"] section[aria-label="Routines"]')
      ?.scrollIntoView({ block: "start" });
  });
}

async function openRoutines() {
  await $('button[aria-label="Agent settings"]').click();
  await scrollToRoutines();
}

function row(name: string) {
  return $(`${settings} li[data-routine="${name}"]`);
}

describe("routines", () => {
  before(async () => {
    await $("button=New agent").click();
    await $("#agent-name").setValue("Routiner");
    await $("button=Create").click();
    await expect($(`${agentView} h2`)).toHaveText("Routiner");
    await openRoutines();
  });

  it("starts with an empty list", async () => {
    await expect($(settings)).toHaveText(
      expect.stringContaining("This agent has no routines yet."),
    );
    await browser.saveScreenshot(join(artifacts, "routines-empty.png"));
  });

  it("shows the time zone, the task hint and the API key notice for a subscription agent", async () => {
    await $("button=New routine").click();
    await expect($(editor)).toBeDisplayed();
    await expect($("#routine-time-zone")).toHaveText("Times are in Asia/Tokyo.");
    await expect($(editor)).toHaveText(
      expect.stringContaining(
        "Each run starts from this message, so write it so it makes sense on its own.",
      ),
    );
    await expect($("#routine-api-key-notice")).toHaveText(
      "Routines run without you. Anthropic expects heavy automated use to use an API key. Consider switching this agent to an API key. Change sign-in",
    );
    await $("#routine-api-key-notice").scrollIntoView({ block: "end" });
    await browser.saveScreenshot(join(artifacts, "routines-editor-notice.png"));
  });

  it("explains what is wrong with a field and keeps the editor open", async () => {
    await $("button=Save routine").click();
    await expect($("#routine-name-error")).toHaveText("Enter a name.");
    await $("#routine-name").setValue("Morning check");
    await $("button=Save routine").click();
    await expect($("#routine-task-error")).toHaveText("Write the task.");
    await $("#routine-task").setValue("Summarise what changed overnight.");
    await $("label=Once").click();
    await setNative("#routine-date", "2020-01-01");
    await $("button=Save routine").click();
    await expect($("#routine-date-error")).toHaveText("Pick a date and time that is still ahead.");
    await browser.saveScreenshot(join(artifacts, "routines-editor-date-past.png"));
    await $("label=Every day").click();
    await expect($("#routine-date")).not.toExist();
    await expect($("#routine-date-error")).not.toExist();
  });

  it("saves a daily routine and lists its schedule in words and its next run", async () => {
    await $("button=Save routine").click();
    await expect($(editor)).not.toExist();
    const created = row("Morning check");
    await expect(created.$('[data-part="description"]')).toHaveText("Every day at 08:00");
    await expect(created.$('[data-part="status"]')).toHaveText(expect.stringMatching(nextText));
    await browser.saveScreenshot(join(artifacts, "routines-daily-created.png"));
  });

  it("pauses to show Paused and resumes to show the next run again", async () => {
    const entry = row("Morning check");
    await entry.$("button=Pause").click();
    await expect(entry.$('[data-part="status"]')).toHaveText("Paused");
    await browser.saveScreenshot(join(artifacts, "routines-paused.png"));
    await entry.$("button=Resume").click();
    await expect(entry.$('[data-part="status"]')).toHaveText(expect.stringMatching(nextText));
  });

  it("edits the schedule to every four hours from 09:30 on weekdays", async () => {
    await row("Morning check").$("button=Edit").click();
    await expect($("#routine-name")).toHaveValue("Morning check");
    await expect($("#routine-task")).toHaveValue("Summarise what changed overnight.");
    await $("label=Every N hours").click();
    await $("#routine-hours").click();
    await $('[role="option"]=4 hours').click();
    await setNative("#routine-time", "09:30");
    await $(`${editor} button[aria-label="Saturday"]`).click();
    await $(`${editor} button[aria-label="Sunday"]`).click();
    await browser.saveScreenshot(join(artifacts, "routines-editor-hourly.png"));
    await $("button=Save routine").click();
    await expect($(editor)).not.toExist();
    const edited = row("Morning check");
    await expect(edited.$('[data-part="description"]')).toHaveText(
      "Every 4 hours from 09:30, on weekdays",
    );
    await expect(edited.$('[data-part="status"]')).toHaveText(expect.stringMatching(nextText));
    await browser.saveScreenshot(join(artifacts, "routines-hourly-edited.png"));
  });

  it("refuses a second routine with the same name, ignoring case", async () => {
    await $("button=New routine").click();
    await $("#routine-name").setValue("MORNING CHECK");
    await $("#routine-task").setValue("Something else");
    await $("button=Save routine").click();
    await expect($("#routine-name-error")).toHaveText(
      "This agent already has a routine with this name.",
    );
    await browser.saveScreenshot(join(artifacts, "routines-name-taken.png"));
  });

  it("takes the user to the sign-in setting from the notice", async () => {
    await $("button=Change sign-in").click();
    await expect($(editor)).not.toExist();
    await browser.waitUntil(
      async () =>
        (await browser.execute(() => document.activeElement?.id ?? "")) === "settings-sign-in",
    );
    await browser.saveScreenshot(join(artifacts, "routines-change-sign-in.png"));
  });

  it("adds a once routine that shows its date", async () => {
    await scrollToRoutines();
    await $("button=New routine").click();
    await $("#routine-name").setValue("Launch day");
    await $("#routine-task").setValue("Announce the launch.");
    await $("label=Once").click();
    await setNative("#routine-date", "2099-12-03");
    await $("button=Save routine").click();
    await expect($(editor)).not.toExist();
    await expect(row("Launch day").$('[data-part="description"]')).toHaveText(
      "Once on 3 Dec 2099 at 08:00",
    );
    await expect(row("Launch day").$('[data-part="status"]')).toHaveText(
      "Next: Thu 3 Dec 2099, 08:00",
    );
  });

  it("removes a routine after asking, and keeps the others", async () => {
    await row("Morning check").$("button=Delete").click();
    await expect($(confirm)).toHaveText(expect.stringContaining("Delete Morning check?"));
    await expect($(confirm)).toHaveText(
      expect.stringContaining("Its run history is removed. Past messages stay in the chat."),
    );
    await browser.saveScreenshot(join(artifacts, "routines-delete-confirm.png"));
    await $(confirm).$("button=Delete").click();
    await expect($(confirm)).not.toExist();
    await expect(row("Morning check")).not.toExist();
    await expect(row("Launch day")).toExist();
    await browser.saveScreenshot(join(artifacts, "routines-deleted.png"));
  });

  it("keeps routines per agent", async () => {
    await browser.keys("Escape");
    await expect($(settings)).not.toExist();
    await $("button=New agent").click();
    await $("#agent-name").setValue("Other");
    await $("button=Create").click();
    await expect($(`${agentView} h2`)).toHaveText("Other");
    await openRoutines();
    await expect($$(`${settings} li[data-routine]`)).toBeElementsArrayOfSize(0);
    await expect($(settings)).toHaveText(
      expect.stringContaining("This agent has no routines yet."),
    );
    await browser.keys("Escape");
    await $(sidebar).$("button*=Routiner").click();
    await openRoutines();
    await expect($$(`${settings} li[data-routine]`)).toBeElementsArrayOfSize(1);
  });
});
