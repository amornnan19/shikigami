import { readFileSync, realpathSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import {
  fakeClaudeCwds,
  fakeClaudeInvocations,
  fakeClaudePrompts,
  fakeClaudeRecordSeparator,
  fakeClaudeStdins,
} from "../fake-providers";
import { pickedFolder, workspacesRoot } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const groupList = 'section[aria-label="Group chats"]';
const agentView = 'section[aria-label="Agent"]';
const groupView = 'section[aria-label="Group"]';
const newGroup = '[role="dialog"]:has(#new-group-description)';
const messageBox = 'textarea[aria-label="Message"]';
const stopButton = 'button[aria-label="Stop"]';
const queue = 'section[aria-label="Queue"]';

async function createAgent(name: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $("button=Create").click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function scrollToEnd(view: string) {
  await browser.execute((selector: string) => {
    const scroller = document.querySelector(`${selector} .overflow-y-auto`);
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }, view);
}

async function sendIn(view: string, text: string) {
  await $(messageBox).setValue(text);
  await browser.keys("Enter");
  await expect($(`${view} ${messageBox}`)).toHaveValue("");
}

async function openGroup() {
  await $(groupList).$("button*=Team").click();
  await expect($(`${groupView} h2`)).toHaveText("Team");
}

async function openAgent(name: string) {
  await $(sidebar).$(`button*=${name}`).click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

function records(path: string) {
  return readFileSync(path, "utf8")
    .split(fakeClaudeRecordSeparator)
    .filter((entry) => entry.length > 0);
}

function invocations() {
  return readFileSync(fakeClaudeInvocations, "utf8").split("\n").filter(Boolean);
}

describe("turns in a group chat", () => {
  before(async () => {
    await createAgent("Ada");
    await createAgent("Bo");
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
  });

  it("answers a message without a mention from the lead, under the lead's name", async () => {
    await expect($(groupView)).toHaveText(expect.stringContaining("No messages yet."));
    await sendIn(groupView, "hello");
    const turn = $(`${groupView} article:nth-of-type(1)`);
    await expect(turn).toHaveText(expect.stringContaining("hello"));
    await expect(turn).toHaveText(expect.stringMatching(/Ada\s*pong$/), { wait: 20000 });
    await expect($(`${groupView} ${messageBox}`)).toHaveAttribute("placeholder", "Message", {
      wait: 10000,
    });
    await browser.saveScreenshot(join(artifacts, "group-turns-answer.png"));
  });

  it("runs the turn in the group folder with the instructions, the briefing and the message", async () => {
    expect(records(fakeClaudeCwds).map((cwd) => cwd.trim())).toEqual([realpathSync(pickedFolder)]);
    expect(records(fakeClaudeStdins)).toEqual(["hello"]);
    const [prompt] = records(fakeClaudePrompts);
    const instructions = readFileSync(join(workspacesRoot, "ada", "CLAUDE.md"), "utf8").trimEnd();
    expect(prompt?.startsWith(instructions)).toBe(true);
    expect(prompt).toContain(
      'You are Ada, a member of the group chat "Team". Members: Ada (you, lead), Bo.',
    );
    expect(prompt).toContain("start a line with @Name followed by the task.");
    const [line] = invocations();
    expect(line).toContain("--session-id");
    expect(line).not.toContain("--resume");
  });

  it("resumes the group session on the next message", async () => {
    await sendIn(groupView, "second message");
    const turn = $(`${groupView} article:nth-of-type(2)`);
    await expect(turn).toHaveText(expect.stringMatching(/Ada\s*pong$/), { wait: 20000 });
    const lines = invocations();
    expect(lines).toHaveLength(2);
    expect(lines[1]).toContain("--resume");
    expect(lines[1]).not.toContain("--session-id");
    expect(records(fakeClaudeStdins)).toEqual(["hello", "second message"]);
  });

  it("keeps the one-to-one chat of the lead separate and on its own session", async () => {
    await openAgent("Ada");
    await expect($(agentView)).toHaveText(expect.stringContaining("No messages yet."));
    await $(messageBox).setValue("hello private");
    await browser.keys("Enter");
    await expect($(`${agentView} article:nth-of-type(1)`)).toHaveText(
      expect.stringMatching(/hello private\s*pong$/),
      { wait: 20000 },
    );
    const lines = invocations();
    expect(lines).toHaveLength(3);
    expect(lines[2]).toContain("--session-id");
    expect(lines[2]).not.toContain("--resume");
    expect(records(fakeClaudePrompts)).toHaveLength(2);
    expect(records(fakeClaudeCwds).at(-1)?.trim()).not.toBe(realpathSync(pickedFolder));
    await browser.saveScreenshot(join(artifacts, "group-turns-private-chat.png"));
  });

  it("stops a running group turn and keeps the partial reply", async () => {
    await openGroup();
    await sendIn(groupView, "SLOW stop me");
    const turn = $(`${groupView} article:nth-of-type(3)`);
    await expect($(stopButton)).toBeEnabled({ wait: 15000 });
    await expect(turn).toHaveText(expect.stringContaining("Counting slowly."), { wait: 15000 });
    await scrollToEnd(groupView);
    await browser.saveScreenshot(join(artifacts, "group-turns-running.png"));
    await $(stopButton).click();
    await expect(turn).toHaveText(expect.stringContaining("Stopped"), { wait: 5000 });
    await expect($(`${groupView} ${messageBox}`)).toHaveAttribute("placeholder", "Message", {
      wait: 5000,
    });
    await scrollToEnd(groupView);
    await browser.saveScreenshot(join(artifacts, "group-turns-stopped.png"));
  });

  it("queues messages in the group, edits and drops them, then runs the next one", async () => {
    await sendIn(groupView, "SLOW first");
    await expect($(stopButton)).toBeDisplayed({ wait: 15000 });
    await sendIn(groupView, "queued one");
    await sendIn(groupView, "queued two");
    await expect($(queue)).toHaveText(expect.stringContaining("Queued (2)"), { wait: 15000 });
    await scrollToEnd(groupView);
    await browser.saveScreenshot(join(artifacts, "group-turns-queue.png"));
    await $(`${queue} li:nth-child(1)`).$("button=Edit").click();
    await $('textarea[aria-label="Edit queued message"]').setValue("queued one edited");
    await $("button=Save").click();
    await expect($(`${queue} li:nth-child(1)`)).toHaveText(
      expect.stringContaining("queued one edited"),
    );
    await $(`${queue} li:nth-child(2) button[aria-label="Remove queued message"]`).click();
    await expect($(queue)).toHaveText(expect.stringContaining("Queued (1)"));
    const next = $(`${groupView} article:nth-of-type(5)`);
    await expect(next).toHaveText(expect.stringContaining("queued one edited"), { wait: 20000 });
    await expect(next).toHaveText(expect.stringMatching(/Ada\s*pong$/), { wait: 20000 });
    await expect($(queue)).not.toExist();
    await expect($(groupView)).not.toHaveText(expect.stringContaining("queued two"));
    await scrollToEnd(groupView);
    await browser.saveScreenshot(join(artifacts, "group-turns-queue-advanced.png"));
  });

  it("holds a group message while the lead works in its own chat, then runs it", async () => {
    await openAgent("Ada");
    await $(messageBox).setValue("SLOW private work");
    await browser.keys("Enter");
    await expect($(stopButton)).toBeEnabled({ wait: 15000 });
    await openGroup();
    await sendIn(groupView, "waits for Ada");
    await expect($(queue)).toHaveText(expect.stringContaining("waits for Ada"), { wait: 15000 });
    await expect($$(`${groupView} article`)).toBeElementsArrayOfSize(5);
    await browser.saveScreenshot(join(artifacts, "group-turns-waits-for-agent.png"));
    const waiting = $(`${groupView} article:nth-of-type(6)`);
    await expect(waiting).toHaveText(expect.stringContaining("waits for Ada"), { wait: 30000 });
    await expect(waiting).toHaveText(expect.stringMatching(/Ada\s*pong$/), { wait: 20000 });
    await expect($(queue)).not.toExist();
    await scrollToEnd(groupView);
    await browser.saveScreenshot(join(artifacts, "group-turns-delayed-ran.png"));
  });

  it("ends the run with a group turn in progress and a message in the queue", async () => {
    await sendIn(groupView, "SLOW unfinished");
    await expect($(stopButton)).toBeDisplayed({ wait: 15000 });
    await sendIn(groupView, "queued before restart");
    await expect($(queue)).toHaveText(expect.stringContaining("queued before restart"), {
      wait: 15000,
    });
    await expect($$(`${groupView} article`)).toBeElementsArrayOfSize(7);
  });
});
