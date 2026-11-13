// What the global setup leaves for the spec and the teardown: the run's folder, the repository and
// home folder in it, the primer the agent session was given and when the story ran. The setup
// writes it to a file in the run's folder and names the file in an environment variable the test
// workers inherit.
import fs from 'node:fs';
import { isRecord, type Place } from './daemon';

/** The environment variable that names the state file. */
export const STATE_VARIABLE = 'CORTEX_E2E_STATE';

/** What one run set up. */
export interface State extends Place {
  /** The run's folder, which holds the repository and the home folder and is deleted at the end. */
  readonly folder: string;
  /** The primer the agent session's start was given, as its hook printed it. */
  readonly primer: string;
  /** When the story began, just before the session's start, in UTC milliseconds. */
  readonly startedMs: number;
  /** When the story ended, once upkeep had run, in UTC milliseconds. */
  readonly endedMs: number;
}

/** The state the setup wrote, or a refusal when there is none or it is not one. */
export function readState(): State {
  const file = process.env[STATE_VARIABLE];
  if (file === undefined) throw new Error(`${STATE_VARIABLE} is not set: the setup did not run`);
  const parsed: unknown = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (
    !isRecord(parsed) ||
    typeof parsed.folder !== 'string' ||
    typeof parsed.root !== 'string' ||
    typeof parsed.home !== 'string' ||
    typeof parsed.primer !== 'string' ||
    typeof parsed.startedMs !== 'number' ||
    typeof parsed.endedMs !== 'number'
  ) {
    throw new Error(`${file} is not the setup's state`);
  }
  return {
    folder: parsed.folder,
    root: parsed.root,
    home: parsed.home,
    primer: parsed.primer,
    startedMs: parsed.startedMs,
    endedMs: parsed.endedMs,
  };
}

/**
 * The UTC dates the story ran on, as the map writes a date (`2026-11-13`), joined as a regular
 * expression's alternatives: one date, or two when the story crossed midnight.
 */
export function storyDays(state: State): string {
  const days = [state.startedMs, state.endedMs].map((ms) =>
    new Date(ms).toISOString().slice(0, 10),
  );
  return [...new Set(days)].join('|');
}
