// The real-daemon checks' global setup: a fresh folder holding a home folder and the scripted
// repository, the daemon and its map started with `cortex map --no-open` as a person starts them,
// the first index and the history miner waited for, then the story played (an agent's task, two
// facts, upkeep). What the spec needs is written to a state file the environment names.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { answerWhere, graphWhere, isRecord, mapAddress, type Place } from './daemon';
import { makeRepository } from './repository';
import { noteFacts, playTask, readHistory } from './story';
import { STATE_VARIABLE, type State } from './state';
import { cleanUp } from './teardown';

/** The files the scripted repository holds, which the first index must list. */
const FILE_COUNT = 6;

/**
 * Makes a fresh folder for one run: under `CORTEX_E2E_DIR` when set (the folder is made if
 * needed), else under the system's temporary folder.
 */
function freshFolder(): string {
  const parent = process.env.CORTEX_E2E_DIR ?? os.tmpdir();
  fs.mkdirSync(parent, { recursive: true });
  return fs.mkdtempSync(path.join(parent, 'cortex-map-e2e-'));
}

/** Waits until the map's health says the first index is done. */
async function indexReady(place: Place): Promise<void> {
  await answerWhere(place, 'api/health', 'the first index', (health) =>
    isRecord(health) && health.index === 'ready' ? true : undefined,
  );
}

/**
 * Sets the repository up, starts the map, plays the story and writes the state for the spec. A
 * setup that fails stops the daemon it started and deletes its folder before it fails.
 */
export default async function globalSetup(): Promise<void> {
  const folder = freshFolder();
  const place: Place = { root: path.join(folder, 'repo'), home: path.join(folder, 'home') };
  const state: State = { folder, ...place, primer: '', startedMs: 0, endedMs: 0 };
  const stateFile = path.join(folder, 'state.json');
  fs.writeFileSync(stateFile, JSON.stringify(state));
  process.env[STATE_VARIABLE] = stateFile;
  try {
    const told = await setUp(place);
    fs.writeFileSync(stateFile, JSON.stringify({ ...state, ...told }));
  } catch (error) {
    await cleanUp(state);
    throw error;
  }
}

/**
 * Makes the repository of `place`, starts its map and waits for its first index and its mined
 * history (the one co-change link), then plays the story; resolves with what the story told and
 * when it ran.
 */
async function setUp(place: Place): Promise<Pick<State, 'primer' | 'startedMs' | 'endedMs'>> {
  fs.mkdirSync(place.home, { recursive: true });
  await makeRepository(place.root, place.home);
  await mapAddress(place);
  await indexReady(place);
  await graphWhere(place, 'the history mined', (map) => {
    return map.files.length === FILE_COUNT && map.linkKinds.includes('co_change');
  });
  const startedMs = Date.now();
  const told = await playTask(place);
  await noteFacts(place);
  await readHistory(place);
  return { primer: told.primer, startedMs, endedMs: Date.now() };
}
