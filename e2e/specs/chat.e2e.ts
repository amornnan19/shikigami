import { readFileSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { fakeClaudeInvocations, fakeClaudeSessionId } from "../fake-providers";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const messageBox = 'textarea[aria-label="Message"]';
const sendButton = 'button[aria-label="Send"]';
const stopButton = 'button[aria-label="Stop"]';
const queue = 'section[aria-label="Queue"]';

async function send(text: string) {
  await expect($(messageBox)).toHaveAttribute("placeholder", "Message");
  await $(messageBox).setValue(text);
  await browser.keys("Enter");
}

async function scrollToEnd() {
  await browser.execute(() => {
    const scroller = document.querySelector('section[aria-label="Agent"] .overflow-y-auto');
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });
}

async function queueMessage(text: string) {
  await expect($(stopButton)).toBeDisplayed();
  await $(messageBox).setValue(text);
  await browser.keys("Enter");
}

function invocations() {
  return readFileSync(fakeClaudeInvocations, "utf8").split("\n").filter(Boolean);
}

describe("chat", () => {
  it("keeps Send disabled while the message is empty", async () => {
    await expect($(sendButton)).toBeDisabled();
    await $(messageBox).setValue("   ");
    await expect($(sendButton)).toBeDisabled();
    await $(messageBox).setValue("");
  });

  it("streams the reply to a message and saves it", async () => {
    await send("Reply with the word pong");
    const turn = $(`${agentView} article:nth-of-type(1)`);
    await expect(turn).toHaveText(expect.stringContaining("Reply with the word pong"));
    await expect(turn).toHaveText(expect.stringMatching(/word pong\s*pong$/));
    await expect($(messageBox)).toHaveValue("");
    await expect($(sendButton)).toBeDisabled();
    await browser.saveScreenshot(join(artifacts, "chat-pong.png"));
  });

  it("shows two failed tool steps collapsed and expands them to their commands", async () => {
    await send("List the files in your folder");
    const turn = $(`${agentView} article:nth-of-type(2)`);
    await turn.waitForExist();
    const toggle = turn.$("button=2 steps");
    await toggle.waitForExist();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(turn).toHaveText(expect.stringContaining("eza"));
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message");
    await expect(turn).not.toHaveText(expect.stringContaining("/bin/ls -la"));
    await browser.saveScreenshot(join(artifacts, "chat-tool-collapsed.png"));
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(turn).toHaveText(expect.stringContaining("Bash"));
    await expect(turn).toHaveText(expect.stringContaining("/bin/ls -la"));
    await expect(turn).toHaveText(expect.stringContaining("denied"));
    await browser.saveScreenshot(join(artifacts, "chat-tool-expanded.png"));
  });

  it("starts with a new session, resumes it afterwards and isolates every turn", async () => {
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(2);
    const lines = invocations();
    expect(lines).toHaveLength(2);
    const [first, second] = lines;
    expect(first).toContain("--session-id");
    expect(first).not.toContain("--resume");
    expect(second).toContain(`--resume ${fakeClaudeSessionId()}`);
    expect(second).not.toContain("--session-id");
    for (const line of lines) {
      expect(line).toContain(
        "--setting-sources  --strict-mcp-config --permission-mode manual --permission-prompts host",
      );
      expect(line).toContain("--permission-prompt-tool mcp__shikigami__approve --mcp-config");
      expect(line).not.toMatch(/bypassPermissions|acceptEdits|dontAsk|dangerously|allowedTools/i);
      expect(line.split(" ")).not.toContain("auto");
      expect(line).not.toMatch(/--setting-sources \S*user/);
      expect(line).not.toContain("pong");
    }
  });

  it("stops a running turn and keeps the partial reply", async () => {
    await send("SLOW stop me");
    const turn = $(`${agentView} article:nth-of-type(3)`);
    await expect($(stopButton)).toBeEnabled({ wait: 15000 });
    await expect(turn).toHaveText(expect.stringContaining("Counting slowly."), { wait: 15000 });
    await expect($(messageBox)).toBeEnabled();
    await expect($(agentView)).toHaveText(
      expect.stringContaining("Messages you send now will wait until the agent finishes."),
    );
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "chat-running-stop.png"));
    await $(stopButton).click();
    await expect(turn).toHaveText(expect.stringContaining("Stopped"), { wait: 3000 });
    await expect(turn).toHaveText(expect.stringMatching(/Counting slowly\..* 60\s*Stopped$/));
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message", { wait: 3000 });
    await expect($(sendButton)).toBeDisabled();
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "chat-stopped.png"));
  });

  it("queues messages during a turn, edits and drops them, then runs the next one", async () => {
    await send("SLOW first");
    await queueMessage("queued one");
    await queueMessage("queued two");
    await expect($(queue)).toHaveText(expect.stringContaining("Queued (2)"), { wait: 15000 });
    await expect($(messageBox)).toHaveValue("");
    await expect($(stopButton)).toBeEnabled();
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "chat-queue.png"));
    await $(`${queue} li:nth-child(1)`).$("button=Edit").click();
    await $('textarea[aria-label="Edit queued message"]').setValue("queued one edited");
    await $("button=Save").click();
    await expect($(`${queue} li:nth-child(1)`)).toHaveText(
      expect.stringContaining("queued one edited"),
    );
    await $(`${queue} li:nth-child(2) button[aria-label="Remove queued message"]`).click();
    await expect($(queue)).toHaveText(expect.stringContaining("Queued (1)"));
    await expect($(queue)).not.toHaveText(expect.stringContaining("queued two"));
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "chat-queue-edited.png"));
    const next = $(`${agentView} article:nth-of-type(5)`);
    await expect(next).toHaveText(expect.stringContaining("queued one edited"), { wait: 20000 });
    await expect($(queue)).not.toExist();
    await expect(next).toHaveText(expect.stringMatching(/edited\s*pong$/), { wait: 20000 });
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message", { wait: 20000 });
    await expect($(agentView)).not.toHaveText(expect.stringContaining("queued two"));
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(5);
    await scrollToEnd();
    await browser.saveScreenshot(join(artifacts, "chat-queue-advanced.png"));
  });

  it("blocks a remote image through the content security policy", async () => {
    const violated = await browser.executeAsync((done: (directive: string) => void) => {
      document.addEventListener(
        "securitypolicyviolation",
        (event) => done(event.violatedDirective),
        {
          once: true,
        },
      );
      const image = new Image();
      image.src = "https://example.invalid/pixel.png";
      document.body.appendChild(image);
    });
    expect(violated).toMatch(/^img-src/);
  });

  it("ends the run with a turn in progress and a message in the queue", async () => {
    await send("SLOW unfinished");
    await queueMessage("queued before restart");
    await expect($(queue)).toHaveText(expect.stringContaining("queued before restart"), {
      wait: 15000,
    });
  });
});
