
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
