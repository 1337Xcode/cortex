// The real-daemon checks' global teardown: stops the repository's daemon with `cortex off`, which
// asks it to stop and waits until it has let go of the repository, then deletes the run's folder.
// A daemon that kept running would hold files there, and the deletion would fail.
import fs from 'node:fs';
import { cortex } from './daemon';
import { readState, STATE_VARIABLE, type State } from './state';

/** Stops the daemon of `state`'s repository, when it was made, and deletes the run's folder. */
export async function cleanUp(state: State): Promise<void> {
  if (fs.existsSync(state.root)) {
    const off = await cortex(state, ['off'], 60_000);
    if (off.code !== 0) throw new Error(`cortex off exited ${String(off.code)}: ${off.stderr}`);
  }
  fs.rmSync(state.folder, { recursive: true, maxRetries: 10, retryDelay: 200 });
}

/**
 * Cleans up the run the setup's state names; fails when the daemon or the folder stays. A setup
 * that failed cleaned up already and left no state file, and there is nothing left to do.
 */
export default async function globalTeardown(): Promise<void> {
  const file = process.env[STATE_VARIABLE];
  if (file === undefined || !fs.existsSync(file)) return;
  await cleanUp(readState());
}
