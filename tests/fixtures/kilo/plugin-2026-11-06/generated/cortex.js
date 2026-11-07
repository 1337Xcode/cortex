// Cortex's plugin for Kilo Code, written by `cortex init`; `cortex uninstall` removes it.
// At each event below it runs `cortex hook` with the event in the JSON Claude Code's hooks use,
// and gives the agent the text Cortex printed, if any. It never throws, blocks or changes a tool
// call: a failure of any kind gives nothing.

import { spawn } from "node:child_process";

/** The Cortex program the plugin runs. */
const CORTEX = "cortex";

/** The agent's name on Cortex's command line. */
const AGENT = "kilo";

/**
 * How long each event's run of `cortex hook` may take before the plugin stops it and gives
 * nothing: the hook's own hard timeout and a little more, so only a hook that hung is stopped.
 */
const KILL_AFTER_MS = { "session.start": 1600, "chat.message": 1100, "tool.execute.before": 400, "tool.execute.after": 600, "experimental.session.compacting": 400 };

/** The most bytes of output the plugin takes from a run; past them it gives nothing. */
const MAX_OUTPUT_BYTES = 65536;

/** The most sessions or calls the plugin keeps text for; the oldest goes first. */
const MAX_KEPT = 256;

/**
 * Runs `cortex hook` for `event` with `payload` on its standard input. Resolves to the text it
 * printed, trimmed, or to "" when it printed nothing, too much, failed or took too long. Never
 * rejects.
 */
function runCortex(event, payload) {
  return new Promise((resolve) => {
    let input;
    let child;
    try {
      input = JSON.stringify({ hook_event_name: event, ...payload });
      child = spawn(CORTEX, ["hook", "--agent", AGENT, "--event", event], {
        stdio: ["pipe", "pipe", "ignore"],
        windowsHide: true,
      });
    } catch {
      resolve("");
      return;
    }
    const chunks = [];
    let size = 0;
    let timer;
    // The text is given before the child is stopped, so a stop that fails holds nothing up.
    const stop = () => {
      clearTimeout(timer);
      resolve("");
      try {
        child.kill();
      } catch {}
    };
    // INVARIANT: fail-open. A run that hangs is stopped, and the agent goes on without its text.
    timer = setTimeout(stop, KILL_AFTER_MS[event] ?? 0);
    // INVARIANT: fail-open. A program that cannot start, or a broken pipe, gives nothing.
    child.on("error", stop);
    child.stdout.on("error", stop);
    child.stdin.on("error", () => {});
    child.stdout.on("data", (chunk) => {
      size += chunk.length;
      // INVARIANT: budgeted. Output past the cap is not kept, and gives nothing.
      if (size > MAX_OUTPUT_BYTES) {
        stop();
        return;
      }
      chunks.push(chunk);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve(code === 0 ? Buffer.concat(chunks).toString("utf8").trim() : "");
    });
    child.stdin.end(input);
  });
}

/** Keeps `text` under `key` in `kept`, dropping the oldest entry past `MAX_KEPT`; "" keeps nothing. */
function keep(kept, key, text) {
  kept.delete(key);
  if (!text) {
    return;
  }
  kept.set(key, text);
  if (kept.size > MAX_KEPT) {
    kept.delete(kept.keys().next().value);
  }
}

/** Takes the text kept under `key` out of `kept`, or "". */
function take(kept, key) {
  const text = kept.get(key) ?? "";
  kept.delete(key);
  return text;
}

/** The texts that are not empty, one paragraph each. */
function joined(...texts) {
  return texts.filter((text) => text).join("\n\n");
}

// The agent's plugin hooks, as OpenCode 1.x and Kilo Code name them. The agent has no event at a
// session's start: the plugin sends one at the first prompt it sees in a session. Text for the
// model goes into the system prompt: the session's primer in every request, a prompt's work order
// in the first request after the prompt only, since its lines were checked against the files
// then; and after a tool's output: a read's hint, a tool's warnings. Events that give nothing back
// are not waited for.

/** Each session's primer, given at its start. */
const primers = new Map();

/** Each session's work order, given at its latest prompt, kept for the next request. */
const orders = new Map();

/** Each read's hint, given before the read, kept for its result. */
const hints = new Map();

/** The sessions the plugin has seen a prompt in. */
const seen = new Map();

/** The text of a user message's parts the user wrote. */
function promptOf(parts) {
  return (parts ?? [])
    .filter((part) => part?.type === "text" && !part.synthetic && typeof part.text === "string")
    .map((part) => part.text)
    .join("\n");
}

/** The plugin: the hooks, for the project in `directory`. */
const server = async ({ directory }) => {
  const at = (sessionID) => ({ session_id: sessionID, cwd: directory });
  return {
    "chat.message": async (input, output) => {
      try {
        const session = input.sessionID;
        if (!seen.has(session)) {
          keep(seen, session, "seen");
          keep(primers, session, await runCortex("session.start", at(session)));
        }
        const prompt = promptOf(output.parts);
        keep(orders, session, await runCortex("chat.message", { ...at(session), prompt }));
      } catch {}
    },
    "experimental.chat.system.transform": async (input, output) => {
      try {
        const text = joined(primers.get(input.sessionID), take(orders, input.sessionID));
        if (text && Array.isArray(output.system)) {
          output.system.push(text);
        }
      } catch {}
    },
    "tool.execute.before": async (input, output) => {
      try {
        if (input.tool !== "read") {
          return;
        }
        const call = { ...at(input.sessionID), tool_name: input.tool, tool_input: output.args };
        keep(hints, input.callID, await runCortex("tool.execute.before", call));
      } catch {}
    },
    "tool.execute.after": async (input, output) => {
      try {
        const call = { ...at(input.sessionID), tool_name: input.tool, tool_input: input.args };
        const warnings = await runCortex("tool.execute.after", call);
        const text = joined(take(hints, input.callID), warnings);
        if (text && typeof output.output === "string") {
          output.output = joined(output.output, text);
        }
      } catch {}
    },
    "experimental.session.compacting": async (input) => {
      try {
        runCortex("experimental.session.compacting", at(input.sessionID));
      } catch {}
    },
  };
};

export default { id: "cortex", server };
