import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { fakeClaudeCwds, fakeClaudeRecordSeparator, fakeClaudeSettings } from "../fake-providers";

const artifacts = join(import.meta.dirname, "..", "artifacts");
const agentView = 'section[aria-label="Agent"]';
const messageBox = 'textarea[aria-label="Message"]';
const sandboxMessage = "The sandbox could not start, so this agent did not run.";

type TurnSettings = {
  sandbox: {
    enabled: boolean;
    failIfUnavailable: boolean;
    allowUnsandboxedCommands: boolean;
    autoAllowBashIfSandboxed: boolean;
    network: { allowedDomains: string[]; strictAllowlist: boolean };
    filesystem: { allowWrite: string[]; denyRead: string[]; denyWrite: string[] };
  };
  permissions: { allow: string[]; deny: string[] };
};

function recordedSettings(): TurnSettings[] {
  return readFileSync(fakeClaudeSettings, "utf8")
    .split(fakeClaudeRecordSeparator)
    .filter((text) => text.trim() !== "")
    .map((text) => JSON.parse(text));
}

function folders(): string[] {
  return readFileSync(fakeClaudeCwds, "utf8").split("\n").filter(Boolean);
}

describe("the sandbox", () => {
  it("runs a turn with the agent's folder writable and the protected places refused", async () => {
    await $("button=New agent").click();
    await $("#agent-name").setValue("Boxed Agent");
    await $("button=Create").click();
    await expect($(`${agentView} h2`)).toHaveText("Boxed Agent");
    await $(messageBox).setValue("Reply with the word pong");
    await browser.keys("Enter");
    const turn = $(`${agentView} article:nth-of-type(1)`);
    await expect(turn).toHaveText(expect.stringMatching(/word pong\s*pong$/), { wait: 20000 });
    const [settings] = recordedSettings();
    const [folder] = folders();
    expect(settings).toBeDefined();
    expect(folder).toBeDefined();
    const sandbox = settings?.sandbox;
    expect(sandbox?.enabled).toBe(true);
    expect(sandbox?.failIfUnavailable).toBe(true);
    expect(sandbox?.allowUnsandboxedCommands).toBe(false);
    expect(sandbox?.autoAllowBashIfSandboxed).toBe(false);
    expect(sandbox?.network).toEqual({ allowedDomains: [], strictAllowlist: true });
    expect(sandbox?.filesystem.allowWrite[0]).toBe(folder);
    const home = homedir();
    for (const place of [".ssh", ".gnupg", ".aws", "Library/Keychains", ".zshrc"]) {
      expect(sandbox?.filesystem.denyRead).toContain(join(home, place));
      expect(sandbox?.filesystem.denyWrite).toContain(join(home, place));
    }
    expect(sandbox?.filesystem.denyRead).not.toContain(folder);
    expect(settings?.permissions.allow).toEqual([`Read(/${folder}/**)`, `Edit(/${folder}/**)`]);
    expect(settings?.permissions.deny).toContain(`Read(/${join(home, ".ssh")}/**)`);
    expect(settings?.permissions.deny).toContain(`Edit(/${join(home, ".ssh")}/**)`);
    await browser.saveScreenshot(join(artifacts, "sandbox-turn.png"));
  });

  it("fails the turn with the sandbox message and holds the agent when the sandbox cannot start", async () => {
    await $(messageBox).setValue("NOSANDBOX please");
    await browser.keys("Enter");
    const turn = $(`${agentView} article:nth-of-type(2)`);
    await expect(turn).toHaveText(expect.stringContaining(sandboxMessage), { wait: 30000 });
    await expect($(messageBox)).toHaveAttribute("placeholder", "Message", { wait: 10000 });
    await expect($('aside[aria-label="Agents"] [aria-label="Needs attention"]')).toBeExisting();
    await expect($$(`${agentView} article`)).toBeElementsArrayOfSize(2);
    await browser.saveScreenshot(join(artifacts, "sandbox-unavailable.png"));
  });
});
