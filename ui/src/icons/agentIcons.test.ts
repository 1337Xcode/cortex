// The agents' icons: each agent the daemon names, the thesvg icon it is drawn with (or none) and
// the product that icon's own title names, written out by hand; and every icon drawn in the
// text's colour alone and taken from the module its name says.
import { describe, expect, it } from 'vitest';
import { agentIcon } from './agentIcons';
import { expectDrawnInTextColour, expectIsThesvgIcon, titleOf } from './testIcons';

/**
 * Every agent's snake name as the daemon sends it, its icon's name in thesvg and the title that
 * icon carries (null where it carries none), or two nulls where the agent has no icon.
 */
const EXPECTED: readonly (readonly [string, string | null, string | null])[] = [
  ['claude', 'claude-code', 'Claude Code'],
  ['codex', 'codex-openai', 'Codex (OpenAI)'],
  ['cursor', 'cursor', 'Cursor'],
  ['gemini', 'google-gemini', 'Google Gemini'],
  ['copilot', 'github-copilot', 'GitHub Copilot'],
  ['qwen', 'qwen', 'Qwen'],
  ['devin', null, null],
  ['windsurf', 'windsurf', 'Windsurf'],
  ['zed', 'zed-industries', 'Zed Industries'],
  ['cline', 'cline', 'Cline'],
  ['roo', 'roocode', 'RooCode'],
  ['aider', null, null],
  ['amp', null, null],
  ['goose', 'goose-codename', 'Goose (codename)'],
  ['kiro', null, null],
  ['trae', 'trae', 'TRAE'],
  ['junie', 'junie', 'Junie'],
  ['augment', null, null],
  ['opencode', 'opencode', 'opencode'],
  ['kilo', 'kilo-code', null],
  ['warp', 'warp', 'Warp'],
  ['continue', null, null],
  ['crush', null, null],
  ['droid', null, null],
  ['openhands', 'openhands', 'OpenHands'],
  ['antigravity', 'antigravity-google', 'Antigravity (Google)'],
  ['hermes', 'nousresearch-hermes', 'NousResearch (Hermes)'],
  ['openclaw', null, null],
  ['other', null, null],
];

describe('the agents', () => {
  it.each(EXPECTED)('draws %s with %s', (agent, name, title) => {
    const icon = agentIcon(agent);
    if (name === null) {
      expect(icon).toBeNull();
      return;
    }
    expect(icon?.name).toBe(name);
    expect(icon === null ? undefined : titleOf(icon)).toBe(title);
  });

  it('gives no icon to a name the daemon does not send', () => {
    for (const name of ['Claude', 'claude-code', '', 'constructor', 'toString']) {
      expect(agentIcon(name), name).toBeNull();
    }
  });

  it.each(EXPECTED.flatMap(([agent, name]) => (name === null ? [] : [agent])))(
    "draws %s's icon in the text's colour alone, from its own module",
    async (agent) => {
      const icon = agentIcon(agent);
      expect(icon).not.toBeNull();
      if (icon === null) return;
      expectDrawnInTextColour(icon);
      await expectIsThesvgIcon(icon);
    },
  );
});
