import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { setFakeClock } from "../fake-clock";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const settings = '[role="dialog"]:has(#agent-settings-description)';
const editor = '[role="dialog"]:has(#routine-editor-description)';
const confirm = '[role="dialog"]:has(#delete-routine-description)';
const marker = '[data-part="routine-marker"]';
const slow = 25000;

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

async function addRoutine(name: string, task: string, once?: { date: string; time: string }) {
  await scrollToRoutines();
  await $("button=New routine").click();
  await $("#routine-name").setValue(name);
  await $("#routine-task").setValue(task);
  if (once) {
    await $("label=Once").click();
    await setNative("#routine-date", once.date);
    await setNative("#routine-time", once.time);
  }
  await $("button=Save routine").click();
  await expect($(editor)).not.toExist();
}

async function scrollChatToEnd() {
  await browser.execute(() => {
    const scroller = document.querySelector('section[aria-label="Agent"] .overflow-y-auto');
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });
}

describe("routine runs", () => {
  before(async () => {
    await $("button=New agent").click();
    await $("#agent-name").setValue("Runner");
    await $("button=Create").click();
    await expect($(`${agentView} h2`)).toHaveText("Runner");
    await openRoutines();
    await addRoutine("Morning", "Check the inbox");
    await addRoutine("Launch", "Announce the launch", { date: "2026-11-29", time: "08:00" });
    await addRoutine("Soon once", "Say hello soon", { date: "2026-11-30", time: "09:00" });
    await addRoutine("Old once", "Say hello early", { date: "2026-11-29", time: "09:00" });
    await addRoutine("Slow check", "SLOW run", { date: "2099-12-03", time: "08:00" });
    await browser.keys("Escape");
    await expect($(settings)).not.toExist();
  });

  it("starts a daily routine at its time as a marked message in the chat", async () => {
    setFakeClock("2026-11-28T08:00:10Z");
    await expect($(`${agentView} article ${marker}`)).toHaveText("Routine: Morning", {
      wait: slow,
    });
    await expect($(`${agentView} article`)).toHaveText(expect.stringContaining("Check the inbox"));
    await expect($(`${agentView} article`)).toHaveText(expect.stringContaining("pong"), {
      wait: slow,
    });
    await scrollChatToEnd();
    await browser.saveScreenshot(join(artifacts, "routine-runs-marker.png"));
    await openRoutines();
    await expect(row("Morning").$('[data-part="last-run"]')).toHaveText(
      "Last run: Completed 08:00",
      { wait: slow },
    );
  });

  it("runs a paused routine now and lists both runs in its history", async () => {
    const entry = row("Morning");
    await entry.$("button=Pause").click();
    await expect(entry.$('[data-part="status"]')).toHaveText("Paused");
    await entry.$("button=Run now").click();
    await entry.$("button=History").click();
    await expect(entry.$$('[data-part="history"] li')).toBeElementsArrayOfSize(2, { wait: slow });
    await expect(entry.$('[data-part="history"]')).toHaveText(
      expect.stringContaining("Run now Completed"),
      { wait: slow },
    );
    await expect(entry.$('[data-part="history"]')).toHaveText(
      expect.stringContaining("Scheduled Completed"),
    );
    await entry.scrollIntoView({ block: "center" });
    await browser.saveScreenshot(join(artifacts, "routine-runs-history.png"));
  });

  it("opens the chat turn of a history entry", async () => {
    await row("Morning").$$('[data-part="history"] li')[1].$("button=Open").click();
    await expect($(settings)).not.toExist();
    await expect($$(`${agentView} article ${marker}`)).toBeElementsArrayOfSize(2);
    await expect($(`${agentView} article ${marker}`)).toBeDisplayedInViewport();
    await openRoutines();
    await row("Morning").$("button=Resume").click();
    await expect(row("Morning").$('[data-part="status"]')).toHaveText(
      "Next: Sun 29 Nov 2026, 08:00",
    );
  });

  it("skips a run while the last one is unfinished and records why", async () => {
    const entry = row("Slow check");
    await entry.$("button=Run now").click();
    await expect(entry.$('[data-part="last-run"]')).toHaveText(expect.stringContaining("Running"), {
      wait: slow,
    });
    await entry.$("button=Run now").click();
    await expect(entry.$('[data-part="last-run"]')).toHaveText(
      "Last run: Skipped: the last run had not finished.",
      { wait: slow },
    );
    await entry.$("button=History").click();
    await entry.scrollIntoView({ block: "center" });
    await browser.saveScreenshot(join(artifacts, "routine-runs-overlap.png"));
    await expect(entry.$('li[data-run="completed"]')).toExist({ wait: slow });
    await expect(entry.$$('[data-part="history"] li')).toBeElementsArrayOfSize(2);
  });

  it("runs a once routine once and shows Done, with a second routine waiting its turn", async () => {
    setFakeClock("2026-11-29T08:00:05Z");
    await expect(row("Launch").$('[data-part="status"]')).toHaveText("Done", { wait: slow });
    await expect(row("Launch").$('[data-part="last-run"]')).toHaveText(
      "Last run: Completed 08:00",
      { wait: slow },
    );
    await expect(row("Morning").$('[data-part="last-run"]')).toHaveText(
      "Last run: Completed 08:00",
      { wait: slow },
    );
    setFakeClock("2026-11-29T08:30:00Z");
    await browser.pause(2000);
    await row("Launch").$("button=History").click();
    await expect(row("Launch").$$('[data-part="history"] li')).toBeElementsArrayOfSize(1);
    await row("Launch").scrollIntoView({ block: "center" });
    await browser.saveScreenshot(join(artifacts, "routine-runs-once-done.png"));
  });

  it("keeps the marker on past turns after the routine is deleted", async () => {
    await row("Launch").$("button=Delete").click();
    await $(confirm).$("button=Delete").click();
    await expect(row("Launch")).not.toExist();
    await browser.keys("Escape");
    await expect($(settings)).not.toExist();
    await scrollChatToEnd();
    await expect($(`${agentView}`)).toHaveText(expect.stringContaining("Routine: Launch"));
    await browser.saveScreenshot(join(artifacts, "routine-runs-deleted-marker.png"));
  });
});
