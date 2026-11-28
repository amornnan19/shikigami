import { existsSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { type ApprovalRequest, approvalStream, interleavedStream } from "../approval-stream";
import { fakeClaudeApprovalMarker, fakeClaudeApprovals, scriptFakeStream } from "../fake-providers";
import { pickedFolder, workspacesRoot } from "../wdio.shared";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const sidebar = 'aside[aria-label="Agents"]';
const groupList = 'section[aria-label="Group chats"]';
const agentView = 'section[aria-label="Agent"]';
const groupView = 'section[aria-label="Group"]';
const newGroup = '[role="dialog"]:has(#new-group-description)';
const messageBox = 'textarea[aria-label="Message"]';
const card = 'section[aria-label="Approval request"]';
const waiting = '[aria-label="Waiting for you"]';

const list: ApprovalRequest = {
  tool: "Bash",
  input: { command: "ls -la", description: "List the files in the folder" },
};

async function createAgent(name: string) {
  await $("button=New agent").click();
  await $("#agent-name").setValue(name);
  await $("button=Create").click();
  await expect($(`${agentView} h2`)).toHaveText(name);
}

async function send(view: string, text: string) {
  await $(`${view} ${messageBox}`).setValue(text);
  await browser.keys("Enter");
  await expect($(`${view} ${messageBox}`)).toHaveValue("");
}

async function turnEnded(view: string) {
  await expect($(`${view} ${messageBox}`)).toHaveAttribute("placeholder", "Message", {
    wait: 20000,
  });
}

async function scrollToEnd(view: string) {
  await browser.execute((selector: string) => {
    const scroller = document.querySelector(`${selector} .overflow-y-auto`);
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }, view);
}

function decisions(): { behavior: string; message?: string; updatedInput?: object }[] {
  return readFileSync(fakeClaudeApprovals, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

describe("approval cards", () => {
  before(async () => {
    scriptFakeStream(1, approvalStream([list]));
    scriptFakeStream(2, approvalStream([list]));
    scriptFakeStream(
      3,
      approvalStream([
        { tool: "Bash", input: { command: "echo first", description: "Say the first word" } },
        { tool: "WebFetch", input: { url: "https://example.com/page" } },
      ]),
    );
    scriptFakeStream(4, approvalStream([list]));
    scriptFakeStream(5, approvalStream([list]));
    scriptFakeStream(6, approvalStream([list]));
    scriptFakeStream(
      7,
      interleavedStream({
        tool: "Write",
        input: { file_path: "/tmp/notes\u202Etxt.md", content: "hello" },
      }),
    );
    scriptFakeStream(8, approvalStream([{ tool: "Bash", input: { command: "touch left-open" } }]));
    await createAgent("Ada");
  });

  it("shows a card for a command and runs it after Allow once", async () => {
    await send(agentView, "Please list the files");
    const open = $(`${agentView} ${card}`);
    await expect(open).toBeDisplayed({ wait: 20000 });
    await expect(open).toHaveText(expect.stringContaining("Ada"));
    await expect(open).toHaveText(expect.stringContaining("wants to run"));
    await expect(open).toHaveText(expect.stringContaining("ls -la"));
    await expect(open).toHaveText(expect.stringContaining("List the files in the folder"));
    expect(await open.$("span[title]").getAttribute("title")).toContain(workspacesRoot);
    const focused = await browser.execute(() => document.activeElement?.tagName);
    expect(focused).toBe("TEXTAREA");
    await expect(open).toHaveText(
      expect.stringContaining("Runs in the sandbox: no network, writes only in ada."),
    );
    await expect(open).toHaveText(
      expect.stringContaining("Ada says: List the files in the folder"),
    );
    await open.$("button=Show full request").click();
    await expect(open.$("[data-approval-detail]")).toHaveText(
      expect.stringContaining('"command": "ls -la"'),
    );
    await open.$("button=Hide full request").click();
    await expect($(`${sidebar} ${waiting}`)).toBeExisting();
    await expect($(`${sidebar} li:nth-child(1)`)).toHaveText(expect.stringContaining("Needs you"));
    expect(existsSync(fakeClaudeApprovalMarker)).toBe(false);
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-card-open.png"));
    await open.$("button=Allow once").click();
    await expect(open).toHaveText(expect.stringContaining("Allowed"));
    await expect(open.$("button=Allow once")).not.toExist();
    await turnEnded(agentView);
    expect(existsSync(fakeClaudeApprovalMarker)).toBe(true);
    const [answer] = decisions();
    expect(answer?.behavior).toBe("allow");
    expect(answer?.updatedInput).toEqual(list.input);
    await expect($(`${sidebar} ${waiting}`)).not.toExist();
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-card-allowed.png"));
  });

  it("refuses the command after Deny and the turn goes on", async () => {
    rmSync(fakeClaudeApprovalMarker, { force: true });
    await send(agentView, "Please list the files again");
    const open = $(`${agentView} article:nth-of-type(2) ${card}`);
    await expect(open).toBeDisplayed({ wait: 20000 });
    await open.$("button=Deny").click();
    await expect(open).toHaveText(expect.stringContaining("Denied"));
    await turnEnded(agentView);
    expect(existsSync(fakeClaudeApprovalMarker)).toBe(false);
    const answer = decisions()[1];
    expect(answer?.behavior).toBe("deny");
    expect(answer?.message).toBe("The user denied this request.");
    await expect($(`${agentView} article:nth-of-type(2)`)).toHaveText(
      expect.stringContaining("The requests were answered."),
    );
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-card-denied.png"));
  });

  it("gives each of two requests its own card, answered one by one", async () => {
    await send(agentView, "Do two things");
    const turn = `${agentView} article:nth-of-type(3)`;
    await expect($$(`${turn} ${card}`)).toBeElementsArrayOfSize(2, { wait: 20000 });
    const first = $$(`${turn} ${card}`)[0];
    const second = $$(`${turn} ${card}`)[1];
    await expect(first).toHaveText(expect.stringContaining("echo first"));
    await expect(second).toHaveText(expect.stringContaining("wants to use"));
    await expect(second).toHaveText(expect.stringContaining("WebFetch https://example.com/page"));
    const reachable = await browser.execute(() => {
      const cards = document.querySelectorAll('section[aria-label="Approval request"]');
      const button = cards[cards.length - 1]?.querySelector("button");
      button?.focus();
      return document.activeElement === button && button?.tabIndex === 0;
    });
    expect(reachable).toBe(true);
    await second.$("button=Allow once").click();
    await expect(second).toHaveText(expect.stringContaining("Allowed"));
    await expect(first.$("button=Allow once")).toBeExisting();
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-two-cards.png"));
    await first.$("button=Deny").click();
    await expect(first).toHaveText(expect.stringContaining("Denied"));
    await turnEnded(agentView);
    const [one, two] = decisions().slice(2);
    expect(one?.behavior).toBe("deny");
    expect(two?.behavior).toBe("allow");
  });

  it("denies a card nobody answers and says it timed out", async () => {
    await send(agentView, "Wait for nobody");
    const open = $(`${agentView} article:nth-of-type(4) ${card}`);
    await expect(open).toBeDisplayed({ wait: 20000 });
    await expect(open).toHaveText(expect.stringContaining("Timed out"), { wait: 30000 });
    await expect(open.$("button=Allow once")).not.toExist();
    await turnEnded(agentView);
    const answer = decisions()[4];
    expect(answer?.behavior).toBe("deny");
    expect(answer?.message).toBe("No answer within 10 minutes, so this request was denied.");
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-card-timed-out.png"));
  });

  it("denies the open card when the turn is stopped", async () => {
    await send(agentView, "Stop while waiting");
    const open = $(`${agentView} article:nth-of-type(5) ${card}`);
    await expect(open).toBeDisplayed({ wait: 20000 });
    await $('button[aria-label="Stop"]').click();
    await expect(open).toHaveText(expect.stringContaining("Denied because the turn stopped"), {
      wait: 20000,
    });
    await expect($(`${agentView} article:nth-of-type(5)`)).toHaveText(
      expect.stringContaining("Stopped"),
    );
    await turnEnded(agentView);
    await expect($(`${sidebar} ${waiting}`)).not.toExist();
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-card-stopped.png"));
  });

  it("shows the card in a group chat with the group folder", async () => {
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
    await send(groupView, "hello team");
    const open = $(`${groupView} ${card}`);
    await expect(open).toBeDisplayed({ wait: 20000 });
    await expect(open).toHaveText(expect.stringContaining("Ada"));
    expect(await open.$("span[title]").getAttribute("title")).toBe(pickedFolder);
    await expect($(`${groupList} ${waiting}`)).toBeExisting();
    await expect($(`${sidebar} section[aria-label="Agent list"] ${waiting}`)).not.toExist();
    await expect(open).toHaveText(expect.stringMatching(/^Awants to run/));
    await expect($(`${groupList} li:nth-child(1)`)).toHaveText(
      expect.stringContaining("Needs you"),
    );
    await browser.saveScreenshot(join(artifacts, "approvals-group-card-open.png"));
    await open.$("button=Allow once").click();
    await expect(open).toHaveText(expect.stringContaining("Allowed"));
    await turnEnded(groupView);
    await expect($(`${groupList} ${waiting}`)).not.toExist();
    await scrollToEnd(groupView);
    await browser.saveScreenshot(join(artifacts, "approvals-group-card-allowed.png"));
  });

  it("places the card where the request happened, escapes hidden characters and shows the full request", async () => {
    await $(sidebar).$("button*=Ada").click();
    await expect($(`${agentView} h2`)).toHaveText("Ada");
    await send(agentView, "Write the notes");
    const turn = $(`${agentView} article:nth-of-type(6)`);
    const open = turn.$(card);
    await expect(open).toBeDisplayed({ wait: 20000 });
    await expect(open).toHaveText(expect.stringContaining("wants to use"));
    await expect(open).toHaveText(expect.stringContaining("Write /tmp/notes\\u202Etxt.md"));
    await expect(open).not.toHaveText(expect.stringContaining("Runs in the sandbox"));
    expect(await open.$("[data-approval-where]").getText()).toBe("/tmp/notes\\u202Etxt.md");
    await open.$("button=Show full request").click();
    await expect(open.$("[data-approval-detail]")).toHaveText(
      expect.stringContaining('"content": "hello"'),
    );
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-interleaved-open.png"));
    await open.$("button=Allow once").click();
    await expect(open).toHaveText(expect.stringContaining("Allowed"));
    await turnEnded(agentView);
    const order = await browser.execute(() => {
      const root = document.querySelector('section[aria-label="Agent"] article:nth-of-type(6)');
      const nodes = Array.from(root?.querySelectorAll("p, section") ?? []);
      const at = (needle: string) =>
        nodes.findIndex((node) => node.textContent?.includes(needle) && node.tagName === "P");
      const cardAt = nodes.findIndex((node) => node.tagName === "SECTION");
      return [at("Before the request."), cardAt, at("After the request.")];
    });
    expect(order[0]).toBeGreaterThanOrEqual(0);
    expect(order[0]).toBeLessThan(order[1] ?? -1);
    expect(order[1]).toBeLessThan(order[2] ?? -1);
    await scrollToEnd(agentView);
    await browser.saveScreenshot(join(artifacts, "approvals-interleaved-answered.png"));
  });

  it("leaves a card open for the restart check", async () => {
    await $(sidebar).$("button*=Ada").click();
    await expect($(`${agentView} h2`)).toHaveText("Ada");
    await send(agentView, "Leave this open");
    const open = $(`${agentView} article:nth-of-type(7) ${card}`);
    await expect(open).toBeDisplayed({ wait: 20000 });
    await expect(open).toHaveText(expect.stringContaining("touch left-open"));
  });
});
