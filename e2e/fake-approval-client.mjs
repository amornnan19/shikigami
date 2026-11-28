import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";

const [configPath, requestsJson] = process.argv.slice(2);
const server = JSON.parse(readFileSync(configPath, "utf8")).mcpServers.shikigami;
const requests = JSON.parse(requestsJson);
const child = spawn(server.command, server.args, { stdio: ["pipe", "pipe", "inherit"] });
const waiting = new Map();
let buffer = "";
let next = 0;

child.stdout.on("data", (chunk) => {
  buffer += chunk.toString();
  for (let end = buffer.indexOf("\n"); end >= 0; end = buffer.indexOf("\n")) {
    const message = JSON.parse(buffer.slice(0, end));
    buffer = buffer.slice(end + 1);
    waiting.get(message.id)?.(message);
  }
});

function ask(method, params) {
  next += 1;
  const id = next;
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return new Promise((resolve) => waiting.set(id, resolve));
}

await ask("initialize", {
  protocolVersion: "2025-11-25",
  capabilities: {},
  clientInfo: { name: "claude-code", version: "fake" },
});
child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);
await ask("tools/list", {});
const answers = await Promise.all(
  requests.map((request, index) =>
    ask("tools/call", {
      name: "approve",
      arguments: { tool_name: request.tool, input: request.input, tool_use_id: `toolu_${index}` },
    }),
  ),
);
for (const answer of answers) {
  console.log(answer.result.content[0].text);
}
child.stdin.end();
