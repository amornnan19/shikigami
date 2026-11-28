import { readFileSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import {
  fakeClaudeInvocations,
  fakeClaudeRecordSeparator,
  fakeClaudeStdins,
  scriptFakeReply,
} from "../fake-providers";
import { pickedFolder } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const groupView = 'section[aria-label="Group"]';
const newGroup = '[role="dialog"]:has(#new-group-description)';
const messageBox = 'textarea[aria-label="Message"]';
const stopButton = 'button[aria-label="Stop"]';
const queue = 'section[aria-label="Queue"]';
const suggestions = '[role="listbox"][aria-label="Members"]';
const limitNotice = "Hand-off limit reached. Mention an agent to continue.";

async function createAgent(name: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $("button=Create").click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function scrollToEnd() {
  await browser.execute((selector: string) => {
    const scroller = document.querySelector(`${selector} .overflow-y-auto`);
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }, groupView);
}

async function idle() {
  await expect($(`${groupView} ${messageBox}`)).toHaveAttribute("placeholder", "Message", {
    wait: 60000,
  });
}

async function send(text: string) {
  await $(messageBox).setValue(text);
  await browser.keys("Enter");
  await expect($(`${groupView} ${messageBox}`)).toHaveValue("");
}

function invocations() {
  return readFileSync(fakeClaudeInvocations, "utf8").split("\n").filter(Boolean).length;
}

function stdins() {
  return readFileSync(fakeClaudeStdins, "utf8")
    .split(fakeClaudeRecordSeparator)
    .filter((entry) => entry.length > 0);
}

function articles() {
  return $$(`${groupView} article`);
}

describe("mentions and hand-offs in a group chat", () => {
  before(async () => {
    await createAgent("Ada");
    await createAgent("Bo");
    await createAgent("Cy");
    await $("button=New group").click();
    await expect($(newGroup)).toBeDisplayed();
    await $("#group-name").setValue("Team");
    await $("button=Choose folder…").click();
    await expect($("#group-folder-path")).toHaveText(pickedFolder);
    await $(newGroup).$("label*=Ada").click();
    await $(newGroup).$("label*=Bo").click();
    await $(newGroup).$("label*=Cy").click();
    await $("#group-lead").click();
    await $('[role="option"]=Ada').click();
    await $("button=Create").click();
    await expect($(`${groupView} h2`)).toHaveText("Team");
  });

  it("suggests the members after @ and filters them by what is typed", async () => {
    await $(messageBox).setValue("@");
    await expect($(suggestions)).toBeDisplayed();
    await expect($$(`${suggestions} [role="option"]`)).toBeElementsArrayOfSize(3);
    await browser.saveScreenshot(join(artifacts, "group-mentions-suggestions.png"));
    await browser.keys("b");
    await expect($$(`${suggestions} [role="option"]`)).toBeElementsArrayOfSize(1);
    await expect($(`${suggestions} [role="option"]`)).toHaveText(expect.stringMatching(/Bo$/));
    await browser.keys("Escape");
    await expect($(suggestions)).not.toExist();
    await browser.keys("Backspace");
    await expect($$(`${suggestions} [role="option"]`)).toBeElementsArrayOfSize(3);
    await browser.keys(["ArrowDown", "Enter"]);
    const value = await browser.execute(
      (selector: string) => (document.querySelector(selector) as HTMLTextAreaElement).value,
      messageBox,
    );
    expect(value).toBe("@Bo ");
    await expect($(suggestions)).not.toExist();
  });

  it("runs the mentioned member and shows the mention as a chip", async () => {
    await browser.keys("list the notes");
    await browser.keys("Enter");
    await expect($(`${groupView} ${messageBox}`)).toHaveValue("");
    const turn = $(`${groupView} article:nth-of-type(1)`);
    await expect(turn).toHaveText(expect.stringMatching(/Bo\s*pong$/), { wait: 20000 });
    await expect(turn.$("[data-mention]")).toHaveText("@Bo");
    await idle();
    await browser.saveScreenshot(join(artifacts, "group-mentions-chip.png"));
  });

  it("hands a task to another member, who answers, and the handing agent then continues", async () => {
    scriptFakeReply(invocations() + 1, "On it.\n@Bo check the README");
    await send("hello");
    await expect(articles()).toBeElementsArrayOfSize(4, { wait: 30000 });
    await idle();
    const [, asked, task, continued] = await articles();
    await expect(asked).toHaveText(expect.stringContaining("On it."));
    await expect(asked.$("[data-hand-off]")).toHaveText(expect.stringContaining("Handed to @Bo"));
    await expect(asked.$("[data-hand-off]")).toHaveText(
      expect.stringContaining("check the README"),
    );
    await expect(task).toHaveText(expect.stringContaining("Task from Ada"));
    await expect(task).toHaveText(expect.stringMatching(/Bo\s*pong$/));
    await expect(continued).toHaveText(expect.stringContaining("Continue with Bo's answer."));
    await expect(continued).toHaveText(expect.stringMatching(/Ada\s*pong$/));
    const last = stdins().at(-1) ?? "";
    expect(last).toContain("Bo: pong");
    expect(last.trimEnd().endsWith("Continue with Bo's answer.")).toBe(true);
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "group-mentions-hand-off.png"));
  });

  it("returns a nested hand-off depth first, each agent continuing for the one above", async () => {
    const before = invocations();
    const shown = await browser.execute(
      (selector: string) => document.querySelectorAll(selector).length,
      `${groupView} article`,
    );
    scriptFakeReply(before + 1, "Delegating.\n@Bo please ask Cy");
    scriptFakeReply(before + 2, "Asking.\n@Cy dig in");
    await send("@Ada nested work");
    await expect(articles()).toBeElementsArrayOfSize(shown + 5, { wait: 30000 });
    await idle();
    expect(invocations()).toBe(before + 5);
    const turns = stdins().slice(-5);
    expect(turns[3]).toContain("Cy: pong");
    expect(turns[3]?.trimEnd().endsWith("Continue with Cy's answer.")).toBe(true);
    expect(turns[4]).toContain("Bo: pong");
    expect(turns[4]?.trimEnd().endsWith("Continue with Bo's answer.")).toBe(true);
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "group-mentions-nested.png"));
  });

  it("stops a chain that keeps handing off after eight agent turns with the limit notice", async () => {
    const before = invocations();
    const targets = ["Bo", "Cy", "Ada", "Bo", "Cy", "Ada", "Bo", "Cy"];
    targets.forEach((name, index) => {
      scriptFakeReply(before + index + 1, `Passing it on.\n@${name} keep going`);
    });
    await send("@Ada start the chain");
    await expect($(groupView)).toHaveText(expect.stringContaining(limitNotice), { wait: 60000 });
    await browser.waitUntil(async () => invocations() === before + 8, { timeout: 60000 });
    await idle();
    await browser.pause(2000);
    expect(invocations()).toBe(before + 8);
    await expect($(queue)).not.toExist();
    const notice = $(groupView).$(`p=${limitNotice}`);
    await notice.scrollIntoView({ block: "center" });
    await expect(notice).toBeDisplayed();
    await browser.saveScreenshot(join(artifacts, "group-mentions-limit.png"));
  });

  it("stops a running hand-off and starts no further turn from it", async () => {
    const before = invocations();
    const shown = await browser.execute(
      (selector: string) => document.querySelectorAll(selector).length,
      `${groupView} article`,
    );
    scriptFakeReply(before + 1, "Starting.\n@Bo SLOW work\n@Cy then check it");
    await send("@Ada begin");
    await expect($(stopButton)).toBeEnabled({ wait: 20000 });
    await expect(articles()).toBeElementsArrayOfSize(shown + 2, { wait: 20000 });
    const handedOn = $(`${groupView} article:nth-of-type(${shown + 2})`);
    await expect(handedOn).toHaveText(expect.stringContaining("Counting slowly."), {
      wait: 20000,
    });
    await scrollToEnd();
    await $(stopButton).click();
    await expect(handedOn).toHaveText(expect.stringContaining("Stopped"), { wait: 10000 });
    await idle();
    await browser.pause(3000);
    expect(invocations()).toBe(before + 2);
    await expect(articles()).toBeElementsArrayOfSize(shown + 2);
    await expect($(queue)).not.toExist();
    const dropped = $(groupView).$("p=Stopped. Hand-offs from this message were dropped.");
    await dropped.scrollIntoView({ block: "center" });
    await expect(dropped).toBeDisplayed();
    await browser.saveScreenshot(join(artifacts, "group-mentions-stopped.png"));
  });
});
