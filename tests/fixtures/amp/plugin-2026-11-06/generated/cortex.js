// Cortex's plugin for Amp, written by `cortex init`; `cortex uninstall` removes it.
// At each event below it runs `cortex hook` with the event in the JSON Claude Code's hooks use,
// and gives the agent the text Cortex printed, if any. It never throws, blocks or changes a tool
// call: a failure of any kind gives nothing.

import { spawn } from "node:child_process";

/** The Cortex program the plugin runs. */
const CORTEX = "cortex";

/** The agent's name on Cortex's command line. */
const AGENT = "amp";

/**
 * How long each event's run of `cortex hook` may take before the plugin stops it and gives
 * nothing: the hook's own hard timeout and a little more, so only a hook that hung is stopped.
 */
const KILL_AFTER_MS = { "session.start": 1600, "agent.start": 1100, "tool.call": 400, "tool.result": 600, "agent.end": 400 };

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

// The plugin's events, as Amp names them. Amp gives a session's start no text: the primer waits
// for the thread's first prompt, and goes with that prompt's work order after the user's message.
// A read's hint, asked before the read, and a tool's warnings go after the tool's output when the
// output is text. A tool call is never answered: Amp decides it as it would without the plugin.
// The end of a turn gives nothing back, and is not waited for.

/** Each thread's primer, given at its start, kept for its first prompt. */
const primers = new Map();

/** Each read's hint, given before the read, kept for its result. */
const hints = new Map();

/** What Amp's result says of how a call ended, where it says. */
function responseOf(status) {
  if (status === "done") {
    return { success: true };
  }
  return status === "error" ? { success: false } : undefined;
}

/** The plugin: its handlers on Amp's events. */
export default function (amp) {
  const at = (event) => ({ session_id: event.thread?.id, cwd: process.cwd() });
  amp.on("session.start", async (event) => {
    try {
      keep(primers, event.thread?.id, await runCortex("session.start", at(event)));
    } catch {}
  });
  amp.on("agent.start", async (event) => {
    try {
      const order = await runCortex("agent.start", { ...at(event), prompt: event.message });
      const text = joined(take(primers, event.thread?.id), order);
      return text ? { message: { content: text } } : undefined;
    } catch {
      return undefined;
    }
  });
  amp.on("tool.call", async (event) => {
    try {
      if (event.tool === "Read") {
        const call = { ...at(event), tool_name: event.tool, tool_input: event.input };
        keep(hints, event.toolUseID, await runCortex("tool.call", call));
      }
    } catch {}
    return undefined;
  });
  amp.on("tool.result", async (event) => {
    try {
      const call = {
        ...at(event),
        tool_name: event.tool,
        tool_input: event.input,
        tool_response: responseOf(event.status),
      };
      const warnings = await runCortex("tool.result", call);
      const text = joined(take(hints, event.toolUseID), warnings);
      if (text && event.status === "done" && typeof event.output === "string") {
        return { status: "done", output: joined(event.output, text) };
      }
    } catch {}
    return undefined;
  });
  amp.on("agent.end", async (event) => {
    try {
      runCortex("agent.end", at(event));
    } catch {}
    return undefined;
  });
}
