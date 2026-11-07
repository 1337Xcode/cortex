
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
