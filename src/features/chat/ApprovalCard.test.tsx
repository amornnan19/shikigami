import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ApprovalCard, folderName, outcomeLabel, visibleText } from "./ApprovalCard";
import type { Approval } from "./api";

const card: Approval = {
  type: "approval",
  id: "p1",
  tool: "Bash",
  summary: "ls -la",
  detail: "{}",
  reason: "List the folder",
  target: null,
  status: "pending",
  answeredBy: null,
  createdAt: 1,
  answeredAt: null,
  folder: "/Users/me/Agents/ada",
};

function html(approval: Approval, showName = true) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <ApprovalCard approval={approval} name="Ada" color="red" showName={showName} />
    </QueryClientProvider>,
  );
}

describe("ApprovalCard", () => {
  it("asks with Allow once and Deny while the request is open", () => {
    const text = html(card);
    expect(text).toContain("wants to run");
    expect(text).toContain("ls -la");
    expect(text).toContain("Ada says: List the folder");
    expect(text).toContain("italic");
    expect(text).toContain("Runs in the sandbox: no network, writes only in ada.");
    expect(text).toContain("Show full request");
    expect(text).toContain("border-amber-500/50");
    expect(text).toContain('title="/Users/me/Agents/ada"');
    expect(text).toContain(">ada<");
    expect(text).toContain("Allow once");
    expect(text).toContain("Deny");
    expect(text).not.toContain("autofocus");
  });

  it("shows the target path of a tool that has one and mutes an answered card", () => {
    const write = {
      ...card,
      tool: "Write",
      summary: "Write /w/a.txt",
      target: "/w/a.txt",
      reason: null,
    };
    const text = html(write);
    expect(text).toContain(">/w/a.txt<");
    expect(text).not.toContain("Runs in the sandbox");
    expect(text).not.toContain("says:");
    const answered = html({ ...write, status: "denied", answeredBy: "user", answeredAt: 5 });
    expect(answered).toContain("bg-muted/30");
    expect(answered).not.toContain("border-amber-500/50");
  });

  it("leaves the agent name out of the header in a group chat", () => {
    expect(html(card, false)).not.toContain('<span class="font-medium">Ada</span>');
    expect(html(card, true)).toContain('<span class="font-medium">Ada</span>');
  });

  it("writes control and direction characters as visible escapes", () => {
    expect(visibleText("ls \u202Eevil\u0007\u200B")).toBe("ls \\u202Eevil\\u0007\\u200B");
    expect(visibleText("a\nb\tc")).toBe("a\nb\tc");
    const text = html({ ...card, summary: "rm \u202Efdp.txt" });
    expect(text).toContain("rm \\u202Efdp.txt");
    expect(text).not.toContain("\u202E");
  });

  it("names the tool for requests that are not shell commands", () => {
    expect(html({ ...card, tool: "WebFetch", summary: "WebFetch https://a.test" })).toContain(
      "wants to use",
    );
  });

  it("replaces the buttons with the outcome and the time once answered", () => {
    const text = html({
      ...card,
      status: "allowed",
      answeredBy: "user",
      answeredAt: 1_700_000_000_000,
    });
    expect(text).not.toContain("Allow once");
    expect(text).toContain("Allowed");
    expect(text).toMatch(/\d{1,2}:\d{2}\s?[AP]M/);
  });

  it("words every outcome", () => {
    expect(outcomeLabel({ ...card, status: "denied", answeredBy: "user" })).toBe("Denied");
    expect(outcomeLabel({ ...card, status: "timed-out", answeredBy: "timeout" })).toBe("Timed out");
    expect(outcomeLabel({ ...card, status: "denied", answeredBy: "stop" })).toBe(
      "Denied because the turn stopped",
    );
    expect(outcomeLabel({ ...card, status: "denied", answeredBy: "relay" })).toBe(
      "Denied because the request was interrupted",
    );
    expect(outcomeLabel({ ...card, status: "denied", answeredBy: "shutdown" })).toBe(
      "Denied because Shikigami closed",
    );
  });

  it("shows the last part of the folder path", () => {
    expect(folderName("/a/b/c/")).toBe("c");
    expect(folderName("/")).toBe("/");
  });
});
