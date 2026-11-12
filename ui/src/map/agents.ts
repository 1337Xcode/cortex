// Which colour family an agent's live marker takes: the agents with a family of their own in the
// tokens (Claude, Codex, Cursor, Gemini) by their snake name, every other agent the shared neutral.
import { AGENT_FAMILIES, type AgentFamily } from '../theme/tokens';

/** The colour family of the agent named `agent` (its snake name, as the live state gives it). */
export function agentFamily(agent: string): AgentFamily {
  return AGENT_FAMILIES.find((family) => family === agent) ?? 'other';
}
