// The words of the daemon's health, as the overview's health section says it: what each part of
// `cortex status` means, in sentences with their numbers. The hook times are the daemon's own
// handling of each request, from its first byte to the answer, not the hook process's whole run,
// and the section says so; it never calls them within or over a budget.
import type { HistoryReader } from '../api/file';
import type { DaemonHealth, GitState, Health } from '../api/health';
import { visibleText } from '../api/guard';
import { countOf, formatCount, formatSpan, formatUtcMoment } from './format';

/** Each hook the daemon knows, as people name it; another is shown by its own name. */
const HOOK_NAMES: Readonly<Record<string, string>> = {
  session_start: 'Session start',
  prompt: 'Prompt',
  pre_tool: 'Before a tool',
  post_tool: 'After a tool',
  compact: 'Compaction',
  stop: 'Stop',
  subagent_start: 'Subagent start',
  subagent_stop: 'Subagent stop',
  session_end: 'Session end',
};

/** A hook's name for the table. */
export function hookName(hook: string): string {
  return HOOK_NAMES[hook] ?? visibleText(hook);
}

/** What the hook table's times are. */
export const HOOK_TIMES_NOTE =
  "How long the daemon took to handle each hook's latest requests: its own handling, from a request's first byte to its answer, not the hook process's whole run.";

/** The miner's state in a sentence. */
const GIT_SENTENCES: Readonly<Record<GitState, string>> = {
  waiting: 'The history miner waits for the first index before it reads any commit.',
  no_repository:
    "The repository's folder is not the top of a git repository, so no history is read.",
  mining: "The history miner is reading the repository's commits.",
  ready: 'The history miner has read the commits up to HEAD as it is now.',
  failed:
    'The history miner failed its last reading; it tries again after a pause, or when HEAD moves.',
  other: 'The history miner is in a state this page does not know.',
};

/** The miner's state, with the daemon's detail when there is one. */
export function gitSentence(daemon: Pick<DaemonHealth, 'git' | 'git_detail'>): string {
  const sentence = GIT_SENTENCES[daemon.git];
  return daemon.git_detail === null ? sentence : `${sentence} ${visibleText(daemon.git_detail)}`;
}

/** The history worker in a sentence. */
export const HISTORY_SENTENCES: Readonly<Record<HistoryReader, string>> = {
  on: "The history worker runs: it reads the commits behind a file's lines when a prompt or upkeep asks.",
  off: 'The history worker is off: history.why and history.seeds are both off.',
  refused: 'The history worker runs, but git is refused in this repository, so it reads nothing.',
};

/** The daemon's version and its first reading, in a sentence. */
export function versionSentence(health: Pick<Health, 'version' | 'index'>): string {
  const reading =
    health.index === 'reading'
      ? 'still reading the repository for the first time'
      : 'finished its first reading of the repository';
  return `Cortex ${visibleText(health.version)}, ${reading}.`;
}

/** The daemon's uptime, dropped events and failed batches, in a sentence. */
export function runSentence(daemon: DaemonHealth): string {
  return `Running for ${formatSpan(daemon.uptime_ms)}; since it started, ${countOf(daemon.events_dropped, 'hook event')} dropped and ${countOf(daemon.failed_batches, 'batch', 'batches')} of events not recorded.`;
}

/** The graph and upkeep, in a sentence. */
export function graphSentence(daemon: DaemonHealth): string {
  const graph =
    daemon.graph === null
      ? 'The graph is not built yet'
      : `The graph holds ${countOf(daemon.graph.nodes, 'node')} and ${countOf(daemon.graph.links, 'link')}, as last built`;
  const upkeep =
    daemon.upkeep_at_ms === null
      ? 'upkeep has not run yet'
      : `upkeep last ran at ${formatUtcMoment(daemon.upkeep_at_ms)}`;
  return `${graph}; ${upkeep}.`;
}

/** One error the daemon kept, in a sentence: its words, how often, and when last. */
export function errorSentence(error: { message: string; count: number; at_ms: number }): string {
  const times = error.count === 1 ? 'once' : `${formatCount(error.count)} times`;
  return `${visibleText(error.message)} (${times}, last at ${formatUtcMoment(error.at_ms)})`;
}

/** What the section's count says: the errors the daemon kept, once the health is known. */
export function errorCount(health: Health): string {
  if (health.daemon.status !== 'read') return 'unknown';
  return countOf(health.daemon.health.errors.length, 'error');
}
