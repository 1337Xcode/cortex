// Runs each generated plugin (the golden copies under `tests/fixtures/<agent>/plugin-2026-11-06/
// generated/`) under Node with a fake host and a fake `cortex`, checks where the plugin puts the
// text it is given back and that no failure of `cortex` reaches the agent, and records what the
// plugin sends at each event as the fixtures under `calls/` beside them. Run from the repository's
// root: `node tests/plugins/record.mjs` records; with `--check` it writes nothing and fails when a
// recorded call differs from its fixture.

import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const S1 = "00000000-0000-4000-8000-000000000001";
const PROMPT = "Fix divide in calc/ops.py";
const TESTS = "python -m pytest -q";
const REPO = "C:\\repo";
const OPS = "C:\\repo\\calc\\ops.py";
const FIXTURES = join(process.cwd(), "tests", "fixtures");
const IS_CHECK = process.argv.includes("--check");

/**
 * The fake `cortex hook`. As `CORTEX_FAKE` says: `answer` records the event and its payload and
 * answers `[cortex] <event>`; `hang` never ends; `fail` answers and exits 3; `partial` answers
 * and never ends; `flood` answers 70,000 bytes.
 */
const FAKE_HOOK = `
const event = process.argv[process.argv.indexOf("--event") + 1];
const mode = process.env.CORTEX_FAKE || "answer";
let input = "";
process.stdin.on("data", (chunk) => (input += chunk));
process.stdin.on("end", () => {
  if (mode === "answer") {
    const call = JSON.stringify({ event, payload: JSON.parse(input) });
    require("node:fs").appendFileSync("calls.jsonl", call + "\\n");
    process.stdout.write("[cortex] " + event + "\\n");
  } else if (mode === "fail") {
    process.stdout.write("[cortex] " + event + "\\n");
    process.exitCode = 3;
  } else if (mode === "flood") {
    process.stdout.write("x".repeat(70000));
  } else {
    if (mode === "partial") {
      process.stdout.write("[cortex] " + event + "\\n");
    }
    setInterval(() => {}, 1000);
  }
});
`;

/** The answer the fake hook gives at `event`. */
const answer = (event) => `[cortex] ${event}`;

/** The calls the fake hook recorded in the current folder, in order. */
function recorded() {
  if (!existsSync("calls.jsonl")) {
    return [];
  }
  return readFileSync("calls.jsonl", "utf8").trim().split("\n").filter((line) => line).map((line) => JSON.parse(line));
}

/**
 * Removes `folder`, waiting up to 10 seconds for a stopped child that ran in it to let it go:
 * Windows does not remove a process's working folder while it is going away.
 */
async function remove(folder) {
  const until = Date.now() + 10000;
  for (;;) {
    try {
      rmSync(folder, { recursive: true, force: true });
      return;
    } catch (error) {
      if (Date.now() > until) {
        throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
}

/** Waits until the fake hook has recorded `count` calls: a handler that does not wait for its run. */
async function settled(count) {
  const until = Date.now() + 10000;
  while (recorded().length < count) {
    assert.ok(Date.now() < until, `waited for ${count} calls, got ${recorded().length}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

/**
 * A folder of its own holding `agent`'s generated plugin `file`, its program `program` (the fake
 * hook run by this Node when left out), and OpenClaw's plugin entry helper; the plugin loaded
 * from it, and the folder, made current.
 */
async function load(agent, file, name, program) {
  const folder = mkdtempSync(join(tmpdir(), `cortex-plugin-${agent}-`));
  writeFileSync(join(folder, "hook"), FAKE_HOOK);
  const generated = join(FIXTURES, agent, "plugin-2026-11-06", "generated", file);
  const text = readFileSync(generated, "utf8").replace(
    'const CORTEX = "cortex";',
    `const CORTEX = ${JSON.stringify(program ?? process.execPath)};`,
  );
  writeFileSync(join(folder, name), text);
  const sdk = join(folder, "node_modules", "openclaw");
  mkdirSync(sdk, { recursive: true });
  writeFileSync(
    join(sdk, "package.json"),
    JSON.stringify({ name: "openclaw", type: "module", exports: { "./plugin-sdk/plugin-entry": "./entry.js" } }),
  );
  writeFileSync(join(sdk, "entry.js"), "export const definePluginEntry = (entry) => entry;\n");
  process.chdir(folder);
  return { folder, module: await import(pathToFileURL(join(folder, name)).href) };
}

/** The handlers of each plugin, by the agent's event names, from its loaded module. */
const PLUGINS = {
  opencode: { file: "cortex.js", handlers: async (module) => module.CortexPlugin({ directory: REPO }) },
  kilo: { file: "cortex.js", handlers: async (module) => module.default.server({ directory: REPO }) },
  amp: { file: "cortex.js", handlers: async (module) => registered((on) => module.default({ on })) },
  openclaw: {
    file: "cortex/index.js",
    handlers: async (module) => registered((on) => module.default.register({ on })),
  },
};

/** The handlers `register` registers through an `on` function. */
function registered(register) {
  const handlers = {};
  register((event, handler) => (handlers[event] = handler));
  return handlers;
}

/** Runs `agent`'s plugin through `drive`, then records or checks the calls it sent. */
async function record(agent, drive) {
  const home = process.cwd();
  process.env.CORTEX_FAKE = "answer";
  const { folder, module } = await load(agent, PLUGINS[agent].file, "plugin.mjs");
  try {
    await drive(await PLUGINS[agent].handlers(module));
    const calls = recorded().map(({ event, payload }) => {
      // The plugin's own working folder is this run's; the fixtures name the repository's.
      if (payload.cwd === folder || payload.cwd === process.cwd()) {
        payload.cwd = REPO;
      }
      return { name: `${event}.json`, payload };
    });
    const out = join(FIXTURES, agent, "plugin-2026-11-06", "calls");
    const named = calls.map(({ name, payload }, index) => [`${String(index + 1).padStart(2, "0")}-${name}`, payload]);
    if (IS_CHECK) {
      const written = readdirSync(out).sort();
      assert.deepEqual(named.map(([name]) => name), written, `${agent}: the calls' names`);
      for (const [name, payload] of named) {
        assert.deepEqual(payload, JSON.parse(readFileSync(join(out, name), "utf8")), `${agent} ${name}`);
      }
    } else {
      rmSync(out, { recursive: true, force: true });
      mkdirSync(out, { recursive: true });
      for (const [name, payload] of named) {
        writeFileSync(join(out, name), JSON.stringify(payload, null, 2) + "\n");
      }
    }
    process.stdout.write(`${agent}: ${calls.length} calls ${IS_CHECK ? "as recorded" : "recorded"}, every text where expected\n`);
  } finally {
    process.chdir(home);
    await remove(folder);
  }
}

/** Drives OpenCode's or Kilo Code's plugin hooks through a session, as the agent calls them. */
async function driveOpencode(hooks) {
  const parts = [{ type: "text", text: PROMPT }, { type: "text", text: "attached", synthetic: true }];
  await hooks["chat.message"]({ sessionID: S1 }, { message: {}, parts });
  const first = { system: ["base"] };
  await hooks["experimental.chat.system.transform"]({ sessionID: S1, model: {} }, first);
  assert.deepEqual(first.system, ["base", `${answer("session.start")}\n\n${answer("chat.message")}`]);
  const second = { system: ["base"] };
  await hooks["experimental.chat.system.transform"]({ sessionID: S1, model: {} }, second);
  assert.deepEqual(second.system, ["base", answer("session.start")]);
  const read = { tool: "read", sessionID: S1, callID: "call_1" };
  await hooks["tool.execute.before"](read, { args: { filePath: OPS } });
  const readOutput = { title: "", output: "def add(a, b):", metadata: {} };
  await hooks["tool.execute.after"]({ ...read, args: { filePath: OPS } }, readOutput);
  assert.equal(readOutput.output, `def add(a, b):\n\n${answer("tool.execute.before")}\n\n${answer("tool.execute.after")}`);
  const bash = { tool: "bash", sessionID: S1, callID: "call_2" };
  await hooks["tool.execute.before"](bash, { args: { command: TESTS } });
  const bashOutput = { title: "", output: "3 passed", metadata: {} };
  await hooks["tool.execute.after"]({ ...bash, args: { command: TESTS } }, bashOutput);
  assert.equal(bashOutput.output, `3 passed\n\n${answer("tool.execute.after")}`);
  const edit = { tool: "edit", sessionID: S1, callID: "call_3", args: {} };
  await hooks["tool.execute.after"](edit, { title: "", output: "Edit applied.", metadata: {} });
  await hooks["experimental.session.compacting"]({ sessionID: S1 }, { context: [] });
  await settled(7);
  await hooks["chat.message"]({ sessionID: S1 }, { message: {}, parts: [{ type: "text", text: "Thanks" }] });
}

/** Drives Amp's plugin handlers through a thread, as Amp calls them. */
async function driveAmp(handlers) {
  const thread = { id: S1 };
  assert.equal(await handlers["session.start"]({ thread }), undefined);
  assert.deepEqual(await handlers["agent.start"]({ thread, message: PROMPT, id: "msg_1" }), {
    message: { content: `${answer("session.start")}\n\n${answer("agent.start")}` },
  });
  const read = { thread, toolUseID: "toolu_1", tool: "Read", input: { path: OPS } };
  assert.equal(await handlers["tool.call"](read), undefined);
  assert.deepEqual(await handlers["tool.result"]({ ...read, status: "done", output: "def add(a, b):" }), {
    status: "done",
    output: `def add(a, b):\n\n${answer("tool.call")}\n\n${answer("tool.result")}`,
  });
  const bash = { thread, toolUseID: "toolu_2", tool: "Bash", input: { cmd: TESTS } };
  assert.equal(await handlers["tool.call"](bash), undefined);
  assert.equal(await handlers["tool.result"]({ ...bash, status: "error", error: "exit 1", output: "1 failed" }), undefined);
  const status = { thread, message: "Done.", id: "msg_2", status: "done", messages: [] };
  assert.equal(await handlers["agent.end"](status), undefined);
  await settled(6);
}

/** Drives OpenClaw's plugin hooks through a session, as its Gateway calls them. */
async function driveOpenClaw(handlers) {
  const ctx = { sessionId: S1, sessionKey: "agent:main:main" };
  await handlers["session_start"]({}, ctx);
  assert.deepEqual(await handlers["before_prompt_build"]({ prompt: PROMPT, messages: [] }, ctx), {
    appendSystemContext: answer("session_start"),
    appendContext: answer("before_prompt_build"),
  });
  assert.deepEqual(await handlers["before_prompt_build"]({ prompt: "Thanks", messages: [] }, ctx), {
    appendContext: answer("before_prompt_build"),
  });
  const observed = [
    ["after_tool_call", { toolName: "read", params: {}, result: { content: [] } }],
    ["after_tool_call", { toolName: "exec", params: {}, error: "exit 1" }],
    ["before_compaction", {}],
    ["agent_end", {}],
    ["session_end", { reason: "idle" }],
  ];
  for (const [index, [hook, event]] of observed.entries()) {
    await handlers[hook](event, ctx);
    await settled(4 + index);
  }
}

/**
 * Whatever goes wrong with `cortex`, a plugin gives the agent nothing and goes on within its kill
 * time: a run that hangs, fails after answering, answers and hangs, answers past the cap, or a
 * program that cannot start. Each prompt's handler, which waits for the most, takes under 5,000
 * ms (the longest kill time is 1,600 ms, a first prompt in OpenCode waits for two).
 */
async function checkFailures() {
  const prompts = {
    opencode: async (handlers) => {
      await handlers["chat.message"]({ sessionID: S1 }, { message: {}, parts: [{ type: "text", text: PROMPT }] });
      const output = { system: [] };
      await handlers["experimental.chat.system.transform"]({ sessionID: S1 }, output);
      return output.system.length ? output.system : undefined;
    },
    kilo: async (handlers) => prompts.opencode(handlers),
    amp: async (handlers) => handlers["agent.start"]({ thread: { id: S1 }, message: PROMPT, id: "msg_1" }),
    openclaw: async (handlers) => handlers["before_prompt_build"]({ prompt: PROMPT, messages: [] }, { sessionId: S1 }),
  };
  const home = process.cwd();
  let loaded = 0;
  for (const [agent, prompt] of Object.entries(prompts)) {
    for (const mode of ["hang", "fail", "partial", "flood", "missing"]) {
      process.env.CORTEX_FAKE = mode;
      const program = mode === "missing" ? join(tmpdir(), "cortex-no-such-program") : undefined;
      loaded += 1;
      const { folder, module } = await load(agent, PLUGINS[agent].file, `plugin-${loaded}.mjs`, program);
      try {
        const handlers = await PLUGINS[agent].handlers(module);
        const started = Date.now();
        const given = await prompt(handlers);
        const elapsed = Date.now() - started;
        assert.equal(given, undefined, `${agent} ${mode}`);
        assert.ok(elapsed < 5000, `${agent} ${mode}: ${elapsed} ms`);
      } finally {
        process.chdir(home);
        await remove(folder);
      }
    }
  }
  process.env.CORTEX_FAKE = "answer";
  process.stdout.write("every failure of cortex gave nothing, within the kill time\n");
}

/** Every handler of every plugin, given nothing, empty objects or nulls, returns without throwing. */
async function checkJunk() {
  const home = process.cwd();
  for (const agent of Object.keys(PLUGINS)) {
    const { folder, module } = await load(agent, PLUGINS[agent].file, "junk.mjs", join(tmpdir(), "cortex-no-such-program"));
    try {
      const handlers = await PLUGINS[agent].handlers(module);
      for (const [event, handler] of Object.entries(handlers)) {
        for (const args of [[], [{}, {}], [null, null]]) {
          await assert.doesNotReject(async () => handler(...args), `${agent} ${event}`);
        }
      }
    } finally {
      process.chdir(home);
      await remove(folder);
    }
  }
  process.stdout.write("every handler took junk without throwing\n");
}

await checkFailures();
await checkJunk();
await record("opencode", driveOpencode);
await record("kilo", driveOpencode);
await record("amp", driveAmp);
await record("openclaw", driveOpenClaw);
