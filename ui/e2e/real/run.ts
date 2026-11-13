// Running the programs the real-daemon checks need, `cortex` and `git`, the way a person's shell
// would: an argument list and never a shell string, standard input given and closed, standard
// output and error read through pipes, and every run bounded by a timeout. Each run sees the
// test's home folder as the user's, reads no git configuration but what is there, and carries none
// of the variables that would make it an agent's shell or point it at another configuration.
import { spawn } from 'node:child_process';
import path from 'node:path';

/** What a finished program left behind. */
export interface Ran {
  /** Its exit code; null when a signal ended it. */
  readonly code: number | null;
  /** What it printed on standard output. */
  readonly stdout: string;
  /** What it printed on standard error. */
  readonly stderr: string;
}

/** How long a program may take unless its caller says otherwise. */
export const RUN_WITHIN_MS = 60_000;

/**
 * How long a program's output may go on after the program has ended. Its pipes close when the
 * last process holding them lets go: the program at once, and a daemon it started never, if the
 * daemon kept them, as an agent reading a hook's output would then find.
 */
export const OUTPUT_ENDS_WITHIN_MS = 20_000;

/**
 * Variables removed from every run: a configuration folder or git setting the person running the
 * checks may have, the Windows home parts that would name their own home, and the markers that make
 * a process an agent's shell (a command the checks run stands for one a person types).
 */
const REMOVED = [
  'CLAUDE_CONFIG_DIR',
  'GIT_CONFIG_GLOBAL',
  'GIT_CONFIG_COUNT',
  'GIT_CONFIG_PARAMETERS',
  'GIT_DIR',
  'GIT_WORK_TREE',
  'GIT_INDEX_FILE',
  'XDG_CONFIG_HOME',
  'HOMEDRIVE',
  'HOMEPATH',
  'CLAUDE_CODE_CHILD_SESSION',
  'CODEX_THREAD_ID',
  'CODEX_TOOL_CALL_ID',
];

/**
 * This process's environment with `home` as the user's home folder (`HOME` and `USERPROFILE`), the
 * system git configuration off and every name of {@link REMOVED} taken out. Names are compared
 * without case, as Windows does.
 */
export function isolated(home: string): Record<string, string> {
  const removed = new Set([...REMOVED, 'HOME', 'USERPROFILE'].map((name) => name.toUpperCase()));
  const env: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value !== undefined && !removed.has(name.toUpperCase())) env[name] = value;
  }
  env.HOME = home;
  env.USERPROFILE = home;
  env.GIT_CONFIG_NOSYSTEM = '1';
  return env;
}

/** Where and how one program runs. */
export interface RunOptions {
  /** The folder it runs in. */
  readonly cwd: string;
  /** Its whole environment. */
  readonly env: Record<string, string>;
  /** What it reads on standard input, which is closed after; nothing when left out. */
  readonly input?: string;
  /** How long it may take, in milliseconds; {@link RUN_WITHIN_MS} when left out. */
  readonly withinMs?: number;
}

/**
 * Runs `program` with `args` as `options` say and resolves with what it left behind, whatever its
 * exit code. Rejects when it cannot start, when it outlasts its time (it is killed first), or when
 * its output goes on {@link OUTPUT_ENDS_WITHIN_MS} past its exit: a process it started holds its
 * pipes. The rejection names the program and its arguments, never what it printed.
 */
export function run(program: string, args: readonly string[], options: RunOptions): Promise<Ran> {
  const name = `${path.basename(program)} ${args.join(' ')}`;
  return new Promise<Ran>((resolve, reject) => {
    const child = spawn(program, [...args], {
      cwd: options.cwd,
      env: options.env,
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
      shell: false,
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout.on('data', (chunk: Buffer) => stdout.push(chunk));
    child.stderr.on('data', (chunk: Buffer) => stderr.push(chunk));
    let exitCode: number | null = null;
    let hasExited = false;
    let isSettled = false;
    const settle = (outcome: () => void): void => {
      if (isSettled) return;
      isSettled = true;
      clearTimeout(runTimer);
      clearTimeout(outputTimer);
      outcome();
    };
    const runTimer = setTimeout(() => {
      child.kill();
      settle(() => {
        reject(new Error(`${name} did not end within ${options.withinMs ?? RUN_WITHIN_MS} ms`));
      });
    }, options.withinMs ?? RUN_WITHIN_MS);
    let outputTimer: ReturnType<typeof setTimeout> | undefined;
    child.on('error', (error) => {
      settle(() => {
        reject(new Error(`${name} could not run: ${error.message}`));
      });
    });
    child.on('exit', (code) => {
      hasExited = true;
      exitCode = code;
      outputTimer = setTimeout(() => {
        settle(() => {
          reject(new Error(`${name} ended, but a process it started still holds its output`));
        });
      }, OUTPUT_ENDS_WITHIN_MS);
    });
    child.on('close', () => {
      if (!hasExited) return;
      settle(() => {
        resolve({
          code: exitCode,
          stdout: Buffer.concat(stdout).toString('utf8'),
          stderr: Buffer.concat(stderr).toString('utf8'),
        });
      });
    });
    // A program may have stopped reading its input already, as a hook that refuses a payload does.
    child.stdin.on('error', () => undefined);
    child.stdin.end(options.input ?? '');
  });
}

/**
 * Runs `program` as {@link run} does and resolves with what it printed on standard output, having
 * exited 0. Rejects otherwise, with its exit code and what it printed on standard error.
 */
export async function succeed(
  program: string,
  args: readonly string[],
  options: RunOptions,
): Promise<string> {
  const ran = await run(program, args, options);
  if (ran.code !== 0) {
    const name = `${path.basename(program)} ${args.join(' ')}`;
    throw new Error(`${name} exited ${String(ran.code)}: ${ran.stderr.trim()}`);
  }
  return ran.stdout;
}

/** Waits `ms` milliseconds. Used only to space two steps apart in time, never to wait for one. */
export function pause(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Asks `condition` again every `everyMs` until it gives a value, and resolves with that value;
 * rejects with `what` once `withinMs` has passed without one.
 */
export async function waitFor<T>(
  what: string,
  condition: () => Promise<T | undefined>,
  withinMs: number,
  everyMs = 200,
): Promise<T> {
  const until = Date.now() + withinMs;
  for (;;) {
    const value = await condition();
    if (value !== undefined) return value;
    if (Date.now() > until) throw new Error(`${what} did not happen within ${withinMs} ms`);
    await pause(everyMs);
  }
}
