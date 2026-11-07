// Cortex's plugin for OpenClaw, written by `cortex init`; `cortex uninstall` removes it.
// At each event below it runs `cortex hook` with the event in the JSON Claude Code's hooks use,
// and gives the agent the text Cortex printed, if any. It never throws, blocks or changes a tool
// call: a failure of any kind gives nothing.

import { spawn } from "node:child_process";

/** The Cortex program the plugin runs. */
const CORTEX = "cortex";

/** The agent's name on Cortex's command line. */
const AGENT = "openclaw";

/**
 * How long each event's run of `cortex hook` may take before the plugin stops it and gives
 * nothing: the hook's own hard timeout and a little more, so only a hook that hung is stopped.
 */
const KILL_AFTER_MS = { "session_start": 1600, "before_prompt_build": 1100, "after_tool_call": 600, "before_compaction": 400, "agent_end": 400, "session_end": 400 };

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

// The plugin's hooks, as OpenClaw names them. OpenClaw's hooks around a tool only observe, so text
// for the model goes with a prompt only: a session's primer before the system prompt's end on the
// session's first prompt, and the prompt's work order after it. Its working folder is its agent
// workspace's, from `OPENCLAW_WORKSPACE_DIR`, which `cortex hook` reads from its environment.
// Hooks that only observe are not waited for.

import { definePluginEntry } from "openclaw/plugin-sdk/plugin-entry";

/** Each session's primer, given at its start, kept for its first prompt. */
const primers = new Map();

/** The session an event's context names. */
function sessionOf(ctx) {
  return ctx?.sessionId ?? ctx?.sessionKey;
}

/** What OpenClaw's result says of how a call ended, where it says: neither field says nothing. */
function responseOf(event) {
  if (typeof event.error === "string" && event.error) {
    return { success: false };
  }
  return event.result === undefined ? undefined : { success: true };
}

export default definePluginEntry({
  id: "cortex",
  name: "Cortex",
  description: "Gives the agent Cortex's map of the repository at each prompt.",
  register(api) {
    api.on("session_start", async (event, ctx) => {
      try {
        const session = sessionOf(ctx);
        keep(primers, session, await runCortex("session_start", { session_id: session }));
      } catch {}
    });
    api.on("before_prompt_build", async (event, ctx) => {
      try {
        const session = sessionOf(ctx);
        const order = await runCortex("before_prompt_build", {
          session_id: session,
          prompt: event.prompt,
        });
        const primer = take(primers, session);
        const added = {};
        if (primer) {
          added.appendSystemContext = primer;
        }
        if (order) {
          added.appendContext = order;
        }
        return primer || order ? added : undefined;
      } catch {
        return undefined;
      }
    });
    api.on("after_tool_call", async (event, ctx) => {
      try {
        runCortex("after_tool_call", {
          session_id: sessionOf(ctx),
          tool_name: event.toolName,
          tool_input: event.params,
          tool_response: responseOf(event),
        });
      } catch {}
    });
    api.on("before_compaction", async (event, ctx) => {
      try {
        runCortex("before_compaction", { session_id: sessionOf(ctx) });
      } catch {}
    });
    api.on("agent_end", async (event, ctx) => {
      try {
        runCortex("agent_end", { session_id: sessionOf(ctx) });
      } catch {}
    });
    api.on("session_end", async (event, ctx) => {
      try {
        runCortex("session_end", { session_id: sessionOf(ctx), reason: event?.reason });
      } catch {}
    });
  },
});
