import { fakeClaudeSessionId } from "./fake-providers";

export interface ApprovalRequest {
  tool: string;
  input: Record<string, unknown>;
}

export function approvalStream(requests: ApprovalRequest[]): (object | string)[] {
  return [
    {
      type: "system",
      subtype: "init",
      tools: ["Read", "Edit", "Write", "Bash"],
      session_id: fakeClaudeSessionId(),
    },
    `#approve ${JSON.stringify(requests)}`,
    {
      type: "assistant",
      message: { id: "m1", content: [{ type: "text", text: "The requests were answered." }] },
    },
    {
      type: "result",
      subtype: "success",
      is_error: false,
      result: "The requests were answered.",
    },
  ];
}
