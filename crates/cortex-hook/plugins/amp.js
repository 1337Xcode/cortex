
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
