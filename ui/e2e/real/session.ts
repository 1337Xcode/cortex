// One Claude Code session as the real-daemon checks play it: the hook payloads Claude Code 2.1.318
// sent, as captured in the repository's fixtures, every path they name under the captured
// repository (`C:\repo`) moved into the test's own, sent to `cortex hook` on standard input as
// Claude Code runs its hooks; and the session's transcript, written line by line in the shape
// Claude Code 2.1.316 wrote, from which the daemon reads the tokens each response took.
import fs from 'node:fs';
import path from 'node:path';
import { CORTEX, isRecord, type Place } from './daemon';
import { isolated, run } from './run';

/** The Claude Code version whose captured hook payloads the session sends. */
const CAPTURED = '2.1.318';

/** The Claude Code version whose captured transcript gives the shape of a response's line. */
const CAPTURED_TRANSCRIPT = '2.1.316';

/** The fixtures of captured agent data, in the repository these checks belong to. */
const FIXTURES = path.join(import.meta.dirname, '..', '..', '..', 'tests', 'fixtures', 'claude');

/** The repository root the captured payloads name. */
const CAPTURED_ROOT = 'C:\\repo';

/** The session the captured payloads come from, which the checks' agent session is. */
export const SESSION = '00000000-0000-4000-8000-000000000001';

/** A captured payload as an object whose fields may be changed. */
export type Payload = Record<string, unknown>;

/**
 * `value` with every string that is a path under the captured repository (the root itself, or a
 * path under it; not a folder whose name only begins the same) moved to the same place under
 * `root`, in this platform's form.
 */
function moved(value: unknown, root: string): unknown {
  if (typeof value === 'string') {
    if (!value.startsWith(CAPTURED_ROOT)) return value;
    const rest = value.slice(CAPTURED_ROOT.length);
    if (rest !== '' && !rest.startsWith('\\')) return value;
    return path.join(root, ...rest.split('\\').filter((name) => name !== ''));
  }
  if (Array.isArray(value)) return value.map((item: unknown) => moved(item, root));
  if (isRecord(value)) {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, moved(item, root)]));
  }
  return value;
}

/** The folder Claude Code keeps the test repository's transcripts in, under `home`. */
export function transcriptsFolder(home: string): string {
  return path.join(home, '.claude', 'projects', 'C--repo');
}

/** The transcript of the session, under `home`. */
export function transcriptOf(home: string): string {
  return path.join(transcriptsFolder(home), `${SESSION}.jsonl`);
}

/**
 * The payload `name` captured from Claude Code {@link CAPTURED}, its paths moved under the
 * repository of `place`, sent as the session's: its id is {@link SESSION} and the transcript it
 * names is the session's under the test's home folder.
 */
export function captured(name: string, place: Place): Payload {
  const parsed: unknown = JSON.parse(fs.readFileSync(path.join(FIXTURES, CAPTURED, name), 'utf8'));
  const payload = moved(parsed, place.root);
  if (!isRecord(payload)) throw new Error(`the payload ${name} is not an object`);
  return { ...payload, session_id: SESSION, transcript_path: transcriptOf(place.home) };
}

/** The native path of the repository file `repoPath` (written with `/`) under `root`. */
export function nativePath(root: string, repoPath: string): string {
  return path.join(root, ...repoPath.split('/'));
}

/**
 * The captured read of a whole file made a read of the whole of `repoPath`, whose text is `text`
 * and which has `lines` lines.
 */
export function readOf(place: Place, repoPath: string, text: string, lines: number): Payload {
  const read = captured('post-tool-use-read.json', place);
  const file = nativePath(place.root, repoPath);
  return {
    ...read,
    tool_input: { file_path: file },
    tool_response: {
      type: 'text',
      file: { filePath: file, content: text, numLines: lines, startLine: 1, totalLines: lines },
    },
  };
}

/**
 * The captured edit made the edit of `repoPath` that replaced the one line `oldLine` (line
 * `lineNumber` of `before`) with `newLine`, as Claude Code reports a one-line edit.
 */
export function editOf(
  place: Place,
  repoPath: string,
  change: { readonly oldLine: string; readonly newLine: string; readonly lineNumber: number },
  before: string,
): Payload {
  const edit = captured('post-tool-use-edit.json', place);
  const file = nativePath(place.root, repoPath);
  return {
    ...edit,
    tool_input: {
      file_path: file,
      old_string: change.oldLine,
      new_string: change.newLine,
      replace_all: false,
    },
    tool_response: {
      filePath: file,
      oldString: change.oldLine,
      newString: change.newLine,
      originalFile: before,
      structuredPatch: [
        {
          oldStart: change.lineNumber,
          oldLines: 1,
          newStart: change.lineNumber,
          newLines: 1,
          lines: [`-${change.oldLine}`, `+${change.newLine}`],
        },
      ],
      userModified: false,
      replaceAll: false,
    },
  };
}

/**
 * The captured run of tests that passed, which Claude Code sent in a session of its own, sent as
 * the session's: its prompt id is the captured prompt's.
 */
export function testsRun(place: Place): Payload {
  const passing = captured('post-tool-use-bash-test-passing.json', place);
  const prompt = captured('user-prompt-submit.json', place);
  return { ...passing, prompt_id: prompt.prompt_id };
}

/** The captured prompt with `words` of its own. */
export function promptOf(place: Place, words: string): Payload {
  return { ...captured('user-prompt-submit.json', place), prompt: words };
}

/**
 * Runs `cortex hook` for `payload` as Claude Code does, in the repository of `place`, and resolves
 * with what it gave the agent to add to its context: null when it printed nothing. Rejects unless
 * the hook ended as Claude Code requires of one: exit code 0, nothing on standard error, and on
 * standard output nothing, or one line holding Claude Code's hook JSON for the event and nothing
 * else.
 */
export async function hook(place: Place, payload: Payload): Promise<string | null> {
  const event = payload.hook_event_name;
  if (typeof event !== 'string') throw new Error('a payload without an event name');
  const ran = await run(CORTEX, ['hook', '--agent', 'claude', '--event', event], {
    cwd: place.root,
    env: isolated(place.home),
    input: JSON.stringify(payload),
    withinMs: 30_000,
  });
  if (ran.code !== 0 || ran.stderr !== '') {
    throw new Error(`the ${event} hook exited ${String(ran.code)}: ${ran.stderr}`);
  }
  if (ran.stdout === '') return null;
  const line = ran.stdout.endsWith('\n') ? ran.stdout.slice(0, -1) : '';
  if (line === '' || line.includes('\n')) throw new Error(`the ${event} hook printed more`);
  const printed: unknown = JSON.parse(line);
  const output = isRecord(printed) ? printed.hookSpecificOutput : undefined;
  const context = isRecord(output) ? output.additionalContext : undefined;
  const isClaudeJson =
    isRecord(printed) &&
    Object.keys(printed).length === 1 &&
    isRecord(output) &&
    Object.keys(output).length === 2 &&
    output.hookEventName === event &&
    typeof context === 'string';
  if (!isClaudeJson) throw new Error(`the ${event} hook printed what Claude Code would not read`);
  return context;
}

/** The tokens one response took, as a transcript records them. */
export interface Usage {
  readonly input: number;
  readonly cacheWrite: number;
  readonly cacheRead: number;
  readonly output: number;
}

/** One response of the session's transcript: when it was written and the tokens it took. */
export interface Response {
  readonly atMs: number;
  readonly usage: Usage;
}

/** The line Claude Code 2.1.316 wrote for a response, as captured: the first in its transcript. */
function capturedResponseLine(): Record<string, unknown> {
  const file = path.join(FIXTURES, CAPTURED_TRANSCRIPT, 'transcripts', 'grep-session.jsonl');
  for (const text of fs.readFileSync(file, 'utf8').split('\n')) {
    if (text === '') continue;
    const line: unknown = JSON.parse(text);
    if (isRecord(line) && line.type === 'assistant' && isRecord(line.message)) return line;
  }
  throw new Error('the captured transcript holds no response');
}

/** The id the `turn`th response of the session gets, its message's and its request's. */
function idOf(kind: 'msg' | 'req' | 'line', turn: number): string {
  const number = String(turn).padStart(4, '0');
  return kind === 'line' ? `00000000-0000-4000-8000-0000000e${number}` : `${kind}_e2e${number}`;
}

/**
 * Writes the session's transcript under the home folder of `place`: one line per response of
 * `responses`, in order, each the captured response line with the session, ids, time and token
 * counts made its own (no cache write lasting an hour, no reasoning tokens).
 */
export function writeTranscript(place: Place, responses: readonly Response[]): void {
  const template = capturedResponseLine();
  const message = template.message;
  if (!isRecord(message)) throw new Error('the captured response has no message');
  const lines = responses.map((response, index) => {
    const turn = index + 1;
    const usage = {
      input_tokens: response.usage.input,
      cache_creation_input_tokens: response.usage.cacheWrite,
      cache_read_input_tokens: response.usage.cacheRead,
      output_tokens: response.usage.output,
      output_tokens_details: { thinking_tokens: 0 },
      server_tool_use: { web_search_requests: 0, web_fetch_requests: 0 },
      service_tier: 'standard',
      cache_creation: {
        ephemeral_1h_input_tokens: 0,
        ephemeral_5m_input_tokens: response.usage.cacheWrite,
      },
      inference_geo: 'global',
      iterations: [],
      speed: 'standard',
      fallback_credit: null,
    };
    return JSON.stringify({
      ...template,
      sessionId: SESSION,
      uuid: idOf('line', turn),
      parentUuid: turn === 1 ? null : idOf('line', turn - 1),
      timestamp: new Date(response.atMs).toISOString(),
      requestId: idOf('req', turn),
      message: { ...message, id: idOf('msg', turn), usage },
    });
  });
  const file = transcriptOf(place.home);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, lines.map((line) => `${line}\n`).join(''));
}
