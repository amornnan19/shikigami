import { readFileSync } from "node:fs";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { fakeClaudeInvocations } from "../fake-providers";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const messageBox = 'textarea[aria-label="Message"]';
const sendButton = 'button[aria-label="Send"]';

async function send(text: string) {
  await expect($(messageBox)).toHaveAttribute("placeholder", "Message");
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

  it("shows tool use as one collapsed step that expands to its command", async () => {
    await send("List the files in your folder");
    const turn = $(`${agentView} article:nth-of-type(2)`);
    await turn.waitForExist();
    const toggle = turn.$("button=1 step");
    await toggle.waitForExist();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(turn).toHaveText(expect.stringContaining("CLAUDE.md"));
    await expect(turn).not.toHaveText(expect.stringContaining("ls -la"));
    await browser.saveScreenshot(join(artifacts, "chat-tool-collapsed.png"));
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(turn).toHaveText(expect.stringContaining("Bash"));
    await expect(turn).toHaveText(expect.stringContaining("ls -la"));
    await browser.saveScreenshot(join(artifacts, "chat-tool-expanded.png"));
  });

  it("starts with a new session, resumes it afterwards and passes no permission flags", async () => {
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(2);
    const lines = invocations();
    expect(lines).toHaveLength(2);
    const [first, second] = lines;
    expect(first).toContain("--session-id");
    expect(first).not.toContain("--resume");
    expect(second).toContain("--resume fa881671-4c21-46dc-a71d-edec9e9f44fd");
    expect(second).not.toContain("--session-id");
    for (const line of lines) {
      expect(line).not.toMatch(/permission|dangerously|allowedTools/i);
      expect(line).not.toContain("pong");
    }
  });
});
