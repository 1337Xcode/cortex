// What happens in the scripted repository before the map is opened, step by step: one agent task
// (a prompt, three reads, an edit Cortex warns about, a passing test run, the read the warning
// asked for, the end of the answer), the tokens of the five responses it took, two facts noted by
// the user, one of which a later change makes suspect, and upkeep run once so that the history
// worker reads the blames of the files the session worked on. Every number the checks expect is
// counted by hand from these steps, next to the step.
import { cortexOut, fileAt, graphWhere, isRecord, panelWhere, type Place } from './daemon';
import {
  DOCS,
  MONEY,
  MUL_BODY,
  MUL_EDITED,
  OPS,
  OPS_CHANGED,
  OPS_EDITED,
  TESTS,
  textOf,
  writeSource,
} from './repository';
import { pause } from './run';
import {
  captured,
  editOf,
  hook,
  promptOf,
  readOf,
  testsRun,
  writeTranscript,
  type Response,
  type Usage,
} from './session';

/** The prompt of the task: vague, so that Cortex gives it no work order (silent when unsure). */
export const PROMPT = 'make it better';

/** The warning the edit of `calc/ops.py` gets: 3 of its 4 commits changed `docs/ops.md` too. */
export const COMPANION_WARNING =
  '[cortex] Edits to calc/ops.py usually come with docs/ops.md (3 of 4 changes); not touched yet.\n';

/** The rule the user notes about `divide`, which the later change to `divide` makes suspect. */
export const DIVIDE_FACT = 'divide rounds its result to cents.';

/** The rule the user notes about `Ledger`, which nothing changes. */
export const LEDGER_FACT = 'Ledger amounts are whole cents.';

/**
 * The tokens of each response of the task, in order; each is written just after the step that
 * names it, so the daemon counts it for the file that step read or edited. The sums by file, by
 * hand: `calc/ops.py` 3,200 + 4,300 = 7,500; `calc/money.py` 2,100; `tests/test_ops.py` 1,100;
 * `docs/ops.md` 550. Input tokens of the task: 3,000 + 1,000 + 2,000 + 4,000 + 500 = 10,500, in
 * 5 turns. Folder `calc`: 7,500 + 2,100 = 9,600.
 */
export const RESPONSES: readonly { readonly after: string; readonly usage: Usage }[] = [
  { after: OPS.path, usage: { input: 3_000, cacheWrite: 0, cacheRead: 0, output: 200 } },
  { after: TESTS.path, usage: { input: 1_000, cacheWrite: 0, cacheRead: 0, output: 100 } },
  { after: MONEY.path, usage: { input: 2_000, cacheWrite: 0, cacheRead: 0, output: 100 } },
  { after: OPS.path, usage: { input: 4_000, cacheWrite: 0, cacheRead: 0, output: 300 } },
  { after: DOCS.path, usage: { input: 500, cacheWrite: 0, cacheRead: 0, output: 50 } },
];

/** What the story left that the checks need and could not know before it ran. */
export interface Told {
  /** The primer the session's start was given, as the hook printed it. */
  readonly primer: string;
}

/**
 * Waits until the map says an agent last read or edited `repoPath` after `afterMs` (or at all,
 * when null), and resolves with that time: the time the daemon stored for the step just sent.
 */
async function touched(place: Place, repoPath: string, afterMs: number | null): Promise<number> {
  const map = await graphWhere(place, `a step on ${repoPath} stored`, (graph) => {
    const at = fileAt(graph, repoPath).touchedMs;
    return at !== null && (afterMs === null || at > afterMs);
  });
  const at = fileAt(map, repoPath).touchedMs;
  if (at === null) throw new Error(`${repoPath} was never touched`);
  return at;
}

/** A step the agent took on a file, and when the response after it was written. */
interface Stepped {
  /** The file it read or edited. */
  readonly path: string;
  /** What the hook gave the agent, if anything. */
  readonly context: string | null;
  /** When the response that follows it was written, in UTC milliseconds. */
  readonly responseMs: number;
}

/**
 * Sends `payload`, a step on `repoPath`, and resolves with the time of the response that follows
 * it: one millisecond after the time the daemon stored for the step. The next step is sent at
 * least 50 ms later, so each response falls between its step and the next.
 */
async function step(
  place: Place,
  payload: Record<string, unknown>,
  repoPath: string,
  last: Map<string, number>,
): Promise<Stepped> {
  const context = await hook(place, payload);
  const at = await touched(place, repoPath, last.get(repoPath) ?? null);
  last.set(repoPath, at);
  await pause(50);
  return { path: repoPath, context, responseMs: at + 1 };
}

/**
 * The responses of {@link RESPONSES}, each at the time of the step it follows; a refusal when the
 * steps taken are not the ones {@link RESPONSES} follows, in order.
 */
function responsesAfter(steps: readonly Stepped[]): Response[] {
  if (steps.length !== RESPONSES.length) throw new Error(`${steps.length} steps were taken`);
  return RESPONSES.map((response, index) => {
    const taken = steps[index];
    if (taken?.path !== response.after)
      throw new Error(`step ${index + 1} is not on ${response.after}`);
    return { atMs: taken.responseMs, usage: response.usage };
  });
}

/**
 * Plays the agent's task in the repository of `place`: the session starts, the prompt comes, the
 * agent reads `calc/ops.py`, its tests and `calc/money.py`, edits `mul` in `calc/ops.py` (and is
 * warned that `docs/ops.md` usually changes with it), runs the tests, which pass, reads
 * `docs/ops.md`, and ends its answer. Resolves with the primer once the daemon has read the
 * transcript and ended the task.
 */
export async function playTask(place: Place): Promise<Told> {
  const primer = await hook(place, captured('session-start-startup.json', place));
  if (primer === null) throw new Error('the session start got no primer');
  const prompted = await hook(place, promptOf(place, PROMPT));
  if (prompted !== null) throw new Error(`the vague prompt got something: ${prompted}`);
  const last = new Map<string, number>();
  const steps: Stepped[] = [];
  for (const source of [OPS, TESTS, MONEY]) {
    const read = readOf(place, source.path, textOf(source), source.lines.length);
    steps.push(await step(place, read, source.path, last));
  }
  writeSource(place.root, OPS_EDITED);
  const mulLine = OPS.lines.indexOf(MUL_BODY) + 1;
  const change = { oldLine: MUL_BODY, newLine: MUL_EDITED, lineNumber: mulLine };
  const edited = await step(place, editOf(place, OPS.path, change, textOf(OPS)), OPS.path, last);
  if (edited.context !== COMPANION_WARNING) {
    throw new Error(`the edit was told ${JSON.stringify(edited.context)}`);
  }
  steps.push(edited);
  const tested = await hook(place, testsRun(place));
  if (tested !== null) throw new Error(`the passing test run was told ${tested}`);
  const docs = readOf(place, DOCS.path, textOf(DOCS), DOCS.lines.length);
  steps.push(await step(place, docs, DOCS.path, last));
  writeTranscript(place, responsesAfter(steps));
  await hook(place, captured('stop.json', place));
  await graphWhere(place, 'the transcript read', (graph) => fileAt(graph, DOCS.path).tokens > 0);
  await panelWhere(place, DOCS.path, 'the warning marked followed', hasUsedTold);
  return { primer };
}

/** Whether `panel` lists, among the times Cortex named its file, one the agent then used. */
function hasUsedTold(panel: Record<string, unknown>): boolean {
  const told = Array.isArray(panel.told) ? panel.told : [];
  return told.some((entry: unknown) => isRecord(entry) && entry.used === true);
}

/**
 * The user notes the two rules (fact 1 about `divide`, fact 2 about `Ledger`), then changes
 * `divide` outside any agent; resolves once the map shows `calc/ops.py` with one suspect fact.
 */
export async function noteFacts(place: Place): Promise<void> {
  const first = await cortexOut(place, ['note', DIVIDE_FACT, '--at', 'calc/ops.py:divide']);
  if (first !== 'fact 1 added\n') throw new Error(`cortex note printed ${first}`);
  const second = await cortexOut(place, [
    'note',
    LEDGER_FACT,
    '--at',
    'calc/money.py:Ledger',
    '--kind',
    'invariant',
  ]);
  if (second !== 'fact 2 added\n') throw new Error(`cortex note printed ${second}`);
  writeSource(place.root, OPS_CHANGED);
  await graphWhere(place, 'the fact on divide suspect', (graph) => {
    return fileAt(graph, OPS.path).suspect === 1;
  });
}

/**
 * Runs upkeep once (`cortex consolidate`), which asks the history worker for the blames of the
 * files the session worked on that have 3 commits or more, and resolves once the panel of
 * `calc/money.py` names the 3 commits behind its lines.
 */
export async function readHistory(place: Place): Promise<void> {
  await cortexOut(place, ['consolidate']);
  await panelWhere(place, MONEY.path, 'the history of calc/money.py read', (panel) => {
    return Array.isArray(panel.history) && panel.history.length === 3;
  });
}
