// The agents working now, as the legend lists them: each agent once with its sessions, the most
// sessions first, then by snake name.
import { describe, expect, it } from 'vitest';
import type { LiveSession } from '../api/live';
import { agentFamily, agentsAtWork } from './agents';

/** A session of `agent` titled `title`, with no files. */
function session(id: string, agent: string, title: string): LiveSession {
  return { id, agent, title, files: [] };
}

describe('the agents at work', () => {
  it('counts each agent once, the most sessions first, then by snake name', () => {
    // codex 2 sessions; aider, claude and cursor 1 each, in snake-name order.
    const sessions = [
      session('s1', 'cursor', 'Cursor'),
      session('s2', 'codex', 'Codex'),
      session('s3', 'claude', 'Claude Code'),
      session('s4', 'aider', 'Aider'),
      session('s5', 'codex', 'Codex'),
    ];
    expect(agentsAtWork(sessions)).toEqual([
      { agent: 'codex', title: 'Codex', sessions: 2 },
      { agent: 'aider', title: 'Aider', sessions: 1 },
      { agent: 'claude', title: 'Claude Code', sessions: 1 },
      { agent: 'cursor', title: 'Cursor', sessions: 1 },
    ]);
    expect(agentsAtWork([...sessions].reverse())).toEqual(agentsAtWork(sessions));
  });

  it('lists no agent while no session runs', () => {
    expect(agentsAtWork([])).toEqual([]);
  });

  it('gives the agents without a colour of their own the shared one', () => {
    expect(['claude', 'codex', 'aider', 'other'].map(agentFamily)).toEqual([
      'claude',
      'codex',
      'other',
      'other',
    ]);
  });
});
