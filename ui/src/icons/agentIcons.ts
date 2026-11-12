// Which icon stands for an agent: its product's mark, or its maker's when the product has none, by
// the agent's snake name as the daemon sends it. Only icons drawn in one colour are used, so an
// agent's mark takes the text's colour beside its name. An agent with no such mark in thesvg has no
// icon, and nothing stands in for one:
// - devin, aider, augment, crush and droid: thesvg has no mark for the product or its maker;
// - amp: thesvg's `amp` is another project's mark, and the maker's mark is drawn only in colour;
// - kiro, continue and openclaw: their marks are drawn only in colour;
// - other: an agent Cortex does not name.
// Each icon is imported by its own path, so the page carries only the icons listed here.
import AntigravityGoogle from '@thesvg/react/antigravity-google';
import ClaudeCode from '@thesvg/react/claude-code';
import Cline from '@thesvg/react/cline';
import CodexOpenai from '@thesvg/react/codex-openai';
import Cursor from '@thesvg/react/cursor';
import GithubCopilot from '@thesvg/react/github-copilot';
import GooseCodename from '@thesvg/react/goose-codename';
import GoogleGemini from '@thesvg/react/google-gemini';
import Junie from '@thesvg/react/junie';
import KiloCode from '@thesvg/react/kilo-code';
import NousresearchHermes from '@thesvg/react/nousresearch-hermes';
import Opencode from '@thesvg/react/opencode';
import Openhands from '@thesvg/react/openhands';
import Qwen from '@thesvg/react/qwen';
import Roocode from '@thesvg/react/roocode';
import Trae from '@thesvg/react/trae';
import Warp from '@thesvg/react/warp';
import Windsurf from '@thesvg/react/windsurf';
import ZedIndustries from '@thesvg/react/zed-industries';
import { brandIcon, type BrandIcon } from './icon';

/**
 * The agents with an icon. Qwen's and Kilo Code's marks have no `mono` variant; their `light`
 * variants draw in one colour, which the icon takes from the text.
 */
const AGENT_ICONS: ReadonlyMap<string, BrandIcon> = new Map([
  ['claude', brandIcon('claude-code', ClaudeCode, 'mono')],
  ['codex', brandIcon('codex-openai', CodexOpenai, 'mono')],
  ['cursor', brandIcon('cursor', Cursor, 'mono')],
  ['gemini', brandIcon('google-gemini', GoogleGemini, 'mono')],
  ['copilot', brandIcon('github-copilot', GithubCopilot, 'mono')],
  ['qwen', brandIcon('qwen', Qwen, 'light')],
  ['windsurf', brandIcon('windsurf', Windsurf, 'mono')],
  ['zed', brandIcon('zed-industries', ZedIndustries, 'mono')],
  ['cline', brandIcon('cline', Cline, 'mono')],
  ['roo', brandIcon('roocode', Roocode, 'mono')],
  ['goose', brandIcon('goose-codename', GooseCodename, 'mono')],
  ['trae', brandIcon('trae', Trae, 'mono')],
  ['junie', brandIcon('junie', Junie, 'mono')],
  ['opencode', brandIcon('opencode', Opencode, 'mono')],
  ['kilo', brandIcon('kilo-code', KiloCode, 'light')],
  ['warp', brandIcon('warp', Warp, 'mono')],
  ['openhands', brandIcon('openhands', Openhands, 'mono')],
  ['antigravity', brandIcon('antigravity-google', AntigravityGoogle, 'mono')],
  ['hermes', brandIcon('nousresearch-hermes', NousresearchHermes, 'mono')],
]);

/** The icon of the agent named `agent` (its snake name), or null when it has none. */
export function agentIcon(agent: string): BrandIcon | null {
  return AGENT_ICONS.get(agent) ?? null;
}
