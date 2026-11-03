// Exercise the extracted connector with actual Pi and an inert loopback model.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join } from "node:path";
import { tmpdir } from "node:os";
import { createInterface } from "node:readline";
import { setTimeout as delay } from "node:timers/promises";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

const options = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, index, all) => {
    if (index % 2 === 0) pairs.push([value, all[index + 1]]);
    return pairs;
  }, []),
);
for (const flag of ["--provider", "--pi", "--output"]) {
  assert.ok(isAbsolute(options[flag] ?? ""), `${flag} requires an absolute path`);
}
const target = `${{ darwin: "darwin", linux: "linux" }[process.platform]}-${{
  arm64: "aarch64", x64: "x86_64",
}[process.arch]}`;
assert.ok(["darwin-aarch64", "darwin-x86_64", "linux-aarch64", "linux-x86_64"].includes(target));
const scratch = await mkdtemp(join(tmpdir(), "pidancr-native-"));
const home = join(scratch, "home");
const agent = join(scratch, "agent");
const cwd = join(scratch, "workspace");
const other = join(scratch, "other");
const sessions = join(scratch, "sessions");
await Promise.all([home, agent, cwd, other, sessions].map((path) => mkdir(path)));
const requests = [];
const sockets = new Set();
const server = createServer(async (request, response) => {
  try {
    assert.equal(request.url, "/v1/chat/completions");
    let body = "";
    for await (const chunk of request) {
      body += chunk;
      assert.ok(body.length <= 1024 * 1024);
    }
    const parsed = JSON.parse(body);
    assert.equal(parsed.model, "deterministic");
    assert.equal(parsed.stream, true);
    requests.push(parsed);
    response.writeHead(200, { "Content-Type": "text/event-stream" });
    const text = ["FIRST", "RESUMED", "HELD", "HELD", "TITLE"][requests.length - 1];
    assert.ok(text, "unexpected model request");
    const chunk = (delta, finish_reason = null, usage) => response.write(
      `data: ${JSON.stringify({
        id: "loopback", object: "chat.completion.chunk", created: 0, model: "deterministic",
        choices: [{ index: 0, delta, finish_reason }], ...(usage ? { usage } : {}),
      })}\n\n`,
    );
    chunk({ role: "assistant", content: "" });
    chunk({ content: text });
    if (text === "HELD") return;
    chunk({}, "stop", { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 });
    response.end("data: [DONE]\n\n");
  } catch (error) {
    response.destroy(error);
  }
});
server.on("connection", (socket) => {
  sockets.add(socket);
  socket.on("close", () => sockets.delete(socket));
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const env = {
  PATH: `${dirname(process.execPath)}:/usr/bin:/bin`,
  HOME: home,
  TMPDIR: scratch,
  PI_COMMAND: options["--pi"],
  PI_CODING_AGENT_DIR: agent,
  PI_SKIP_VERSION_CHECK: "1",
};
const args = [
  "--provider", "loopback", "--model", "deterministic",
  "--no-extensions", "--no-skills", "--no-prompt-templates", "--session-dir", sessions,
];
const model = "loopback/deterministic";
const initialize = { protocolVersion: 1, clientCapabilities: {}, clientInfo: { name: "native-qa", version: "1" } };
await writeFile(join(agent, "models.json"), JSON.stringify({
  providers: {
    loopback: {
      api: "openai-completions", baseUrl: `http://127.0.0.1:${server.address().port}/v1`,
      apiKey: "inert-local-fixture",
      models: [{ id: "deterministic", reasoning: false, input: ["text"], contextWindow: 32768,
        maxTokens: 512, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }],
    },
  },
}));

function start() {
  const child = spawn(options["--provider"], ["run", "--protocol", "acp-v1", ...args], {
    cwd, env, stdio: ["pipe", "pipe", "pipe"],
  });
  const updates = [];
  const pending = new Map();
  let nextId = 0;
  const lines = createInterface({ input: child.stdout });
  const fail = (error) => {
    for (const request of pending.values()) request.reject(error);
    pending.clear();
  };
  child.on("error", fail);
  child.on("exit", () => fail(new Error("connector exited")));
  child.stderr.resume();
  lines.on("line", (line) => {
    try {
      const message = JSON.parse(line);
      if (message.method === "session/update") updates.push(message.params.update);
      else if (message.method) throw new Error(`unexpected server request: ${message.method}`);
      else {
        const request = pending.get(message.id);
        if (!request) return;
        pending.delete(message.id);
        if (message.error) request.reject(Object.assign(new Error(message.error.message), { rpc: message.error }));
        else request.resolve(message.result);
      }
    } catch (error) { fail(error); }
  });
  return {
    updates,
    request(method, params) {
      return new Promise((resolve, reject) => {
        const id = ++nextId;
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 20_000);
        pending.set(id, {
          resolve: (value) => { clearTimeout(timer); resolve(value); },
          reject: (error) => { clearTimeout(timer); reject(error); },
        });
        child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
      });
    },
    notify(method, params) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
    },
    async stop() {
      if (child.exitCode !== null) return;
      const closed = new Promise((resolve) => child.once("exit", resolve));
      const timer = setTimeout(() => child.kill(), 5_000);
      child.stdin.end();
      await closed;
      clearTimeout(timer);
      lines.close();
    },
  };
}

async function turn(client, sessionId, text, expected) {
  const offset = client.updates.length;
  const result = await client.request("session/prompt", { sessionId, prompt: [{ type: "text", text }] });
  assert.equal(result.stopReason, "end_turn");
  const updates = client.updates.slice(offset);
  assert.equal(updates.filter((update) => update.sessionUpdate === "agent_message_chunk")
    .map((update) => update.content.text).join(""), expected);
  assert.ok(updates.some((update) => update.sessionUpdate === "usage_update" && update.used === 2 && update.size === 32768));
}

let active;
try {
  const piVersion = execFileSync(options["--pi"], ["--version"], { cwd, env, encoding: "utf8", timeout: 15_000 }).trim();
  assert.equal(piVersion, "0.85.1");
  const version = execFileSync(options["--provider"], ["version"], { encoding: "utf8" }).trim();
  const catalog = JSON.parse(execFileSync(options["--provider"],
    ["models", "--format", "acp-config-options-v1", "--cwd", cwd, ...args],
    { cwd, env, encoding: "utf8", timeout: 10_000 }));
  assert.deepEqual(catalog[0].options.map((option) => option.value), [model]);
  assert.equal(requests.length, 0);
  active = start();
  const incompatible = await active.request("initialize", { protocolVersion: 2 });
  assert.equal(incompatible.protocolVersion, 1);
  assert.deepEqual(incompatible.agentCapabilities, {});
  const initialized = await active.request("initialize", initialize);
  assert.equal(initialized.agentInfo.name, "pidancr");
  assert.equal(initialized.agentInfo.title, "Pidancr");
  assert.equal(initialized.agentInfo.version, version);
  assert.deepEqual(initialized.agentCapabilities.sessionCapabilities, { resume: {}, close: {} });
  const empty = await active.request("session/new", { cwd, mcpServers: [] });
  assert.ok(empty.sessionId.startsWith("pidancr-v1."));
  assert.deepEqual(empty.configOptions, catalog);
  await active.request("session/set_config_option", { sessionId: empty.sessionId, configId: "model", value: model });
  await active.request("session/close", { sessionId: empty.sessionId });
  await active.stop();
  active = start();
  await active.request("initialize", initialize);
  await assert.rejects(active.request("session/resume", { sessionId: empty.sessionId, cwd }),
    (error) => error.rpc?.code === -32602 && error.rpc.data?.message === `Session "${empty.sessionId}" not found`);
  assert.equal(requests.length, 0);
  const session = await active.request("session/new", { cwd, mcpServers: [] });
  await turn(active, session.sessionId, "Remember LOCAL_MEMORY_7.", "FIRST");
  await active.stop();
  active = start();
  await active.request("initialize", initialize);
  await assert.rejects(active.request("session/resume", { sessionId: session.sessionId, cwd: other }),
    /another workspace/);
  const resumed = await active.request("session/resume", { sessionId: session.sessionId, cwd });
  assert.deepEqual(resumed.configOptions, catalog);
  await assert.rejects(active.request("session/resume", { sessionId: session.sessionId, cwd }),
    (error) => error.rpc?.data?.message.includes("already active"));
  await turn(active, session.sessionId, "Continue.", "RESUMED");
  assert.ok(JSON.stringify(requests[1].messages).includes("LOCAL_MEMORY_7"), "resumed context was lost");
  const pending = active.request("session/prompt", { sessionId: session.sessionId, prompt: [{ type: "text", text: "Hold." }] });
  const deadline = Date.now() + 10_000;
  while (requests.length < 3 && Date.now() < deadline) await delay(20);
  assert.equal(requests.length, 3);
  active.notify("session/cancel", { sessionId: session.sessionId });
  assert.equal((await pending).stopReason, "cancelled");
  const closing = active.request("session/prompt", {
    sessionId: session.sessionId, prompt: [{ type: "text", text: "Close while active." }],
  });
  const closeDeadline = Date.now() + 10_000;
  while (requests.length < 4 && Date.now() < closeDeadline) await delay(20);
  assert.equal(requests.length, 4);
  await active.request("session/close", { sessionId: session.sessionId });
  assert.equal((await closing).stopReason, "cancelled");
  const naming = await active.request("session/new", {
    cwd, mcpServers: [], _meta: { "com.cadencr": { purpose: "auto_name", systemPrompt: "Return a title." } },
  });
  await turn(active, naming.sessionId, "Name this session.", "TITLE");
  await active.request("session/close", { sessionId: naming.sessionId });
  const providerSha256 = createHash("sha256").update(await readFile(options["--provider"])).digest("hex");
  await writeFile(options["--output"], JSON.stringify({
    passed: true, target, version, provider_id: initialized.agentInfo.name,
    pi_version: piVersion, provider_sha256: providerSha256,
    checks: ["identity", "discovery", "ACP v1", "model selection", "empty-session absence", "streaming", "usage",
      "durable resume", "workspace isolation", "duplicate resume rejection", "cancellation",
      "close while prompting", "auto naming"],
  }, null, 2) + "\n", { flag: "wx" });
  console.log(`Native QA passed: ${target}, Pi ${piVersion}, connector ${version}; 5 local model requests.`);
} finally {
  await active?.stop();
  for (const socket of sockets) socket.destroy();
  await new Promise((resolve) => server.close(resolve));
}
