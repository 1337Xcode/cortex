// Which colour family an agent's live marker takes: the agents with a family of their own in the
// tokens (Claude, Codex, Cursor, Gemini) by their snake name, every other agent the shared neutral.
// And which agents are working now, for the legend's entry per agent.
import type { LiveSession } from '../api/live';
import { AGENT_FAMILIES, type AgentFamily } from '../theme/tokens';

/** The colour family of the agent named `agent` (its snake name, as the live state gives it). */
export function agentFamily(agent: string): AgentFamily {
  return AGENT_FAMILIES.find((family) => family === agent) ?? 'other';
}

/** An agent with sessions running now. */
export interface AgentAtWork {
  /** Its snake name. */
  readonly agent: string;
  /** Its display name, as its first session gives it. */
  readonly title: string;
  /** How many of its sessions run. */
  readonly sessions: number;
}

/**
 * The agents of `sessions`, each once with how many sessions it runs: the most sessions first,
 * then by snake name, so the same sessions always list in the same order.
 */
export function agentsAtWork(sessions: readonly LiveSession[]): readonly AgentAtWork[] {
  const byAgent = new Map<string, AgentAtWork>();
  for (const session of sessions) {
    const known = byAgent.get(session.agent);
    byAgent.set(session.agent, {
      agent: session.agent,
      title: known?.title ?? session.title,
      sessions: (known?.sessions ?? 0) + 1,
    });
  }
  return [...byAgent.values()].sort(
    (a, b) => b.sessions - a.sessions || (a.agent < b.agent ? -1 : a.agent > b.agent ? 1 : 0),
  );
}
