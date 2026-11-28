import { fakeClaudeSessionId } from "./fake-providers";

const toolUse = (messageId: string, id: string, name: string, input: object, parent?: string) => ({
  type: "assistant",
  ...(parent ? { parent_tool_use_id: parent } : {}),
  message: { id: messageId, content: [{ type: "tool_use", id, name, input }] },
});

const toolResult = (id: string, content: string, isError: boolean, parent?: string) => ({
  type: "user",
  ...(parent ? { parent_tool_use_id: parent } : {}),
  message: {
    content: [{ type: "tool_result", tool_use_id: id, content, is_error: isError }],
  },
});

export function subagentStream(holdSeconds: number): (object | string)[] {
  return [
    {
      type: "system",
      subtype: "init",
      tools: ["Read", "Edit", "Write", "Bash"],
      session_id: fakeClaudeSessionId(),
    },
    toolUse("m1", "agent-a", "Agent", {
      subagent_type: "Explore",
      description: "Scan the notes",
      prompt:
        "Read notes.txt and summarise it. Keep the summary short, name every heading you find, list any open questions, and say which section looks the least finished so the next person knows where to start. Do not edit any file. Check the notes for dates and owners as well, and compare the wording of the opening paragraph with the closing one, because the two were written by different people and may disagree. Quote anything that looks contradictory, and finish with a one line verdict on whether the notes are ready to share.",
    }),
    toolUse("m2", "agent-b", "Agent", {
      subagent_type: "Plan",
      description: "Draft the outline",
      prompt: "Plan the outline.",
    }),
    toolUse("m3", "inner-a", "Read", { file_path: "notes.txt" }, "agent-a"),
    toolUse("m4", "inner-b", "Bash", { command: "ls -la" }, "agent-b"),
    toolResult("inner-a", "group notes", false, "agent-a"),
    `#sleep ${holdSeconds}`,
    toolResult("inner-b", "permission denied", true, "agent-b"),
    toolResult("agent-a", "Notes scanned.", false),
    "#sleep 1",
    toolResult("agent-b", "Could not draft the outline.", true),
    {
      type: "assistant",
      message: {
        id: "m5",
        content: [{ type: "text", text: "The scan is done, the outline failed." }],
      },
    },
    {
      type: "result",
      subtype: "success",
      is_error: false,
      result: "The scan is done, the outline failed.",
    },
  ];
}
