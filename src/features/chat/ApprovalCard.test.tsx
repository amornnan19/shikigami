import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ApprovalCard, folderName, outcomeLabel } from "./ApprovalCard";
import type { Approval } from "./api";

const card: Approval = {
  type: "approval",
  id: "p1",
  tool: "Bash",
  summary: "ls -la",
  detail: "{}",
  reason: "List the folder",
  status: "pending",
  answeredBy: null,
  createdAt: 1,
  answeredAt: null,
  folder: "/Users/me/Agents/ada",
};

function html(approval: Approval) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <ApprovalCard approval={approval} name="Ada" color="red" />
    </QueryClientProvider>,
  );
}

describe("ApprovalCard", () => {
  it("asks with Allow once and Deny while the request is open", () => {
    const text = html(card);
    expect(text).toContain("wants to run");
    expect(text).toContain("ls -la");
    expect(text).toContain("List the folder");
    expect(text).toContain('title="/Users/me/Agents/ada"');
    expect(text).toContain(">ada<");
    expect(text).toContain("Allow once");
    expect(text).toContain("Deny");
    expect(text).not.toContain("autofocus");
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
    expect(outcomeLabel({ ...card, status: "denied", answeredBy: "shutdown" })).toBe(
      "Denied because Shikigami closed",
    );
  });

  it("shows the last part of the folder path", () => {
    expect(folderName("/a/b/c/")).toBe("c");
    expect(folderName("/")).toBe("/");
  });
});
