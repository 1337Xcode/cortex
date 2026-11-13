// The real Cortex binary as the checks drive it: where it is, `cortex <command> --root <repo>` run
// as a person types it, the map's address as `cortex map --no-open` prints it, and the map's API
// read over HTTP with nothing but the address (the token is in it), as the page reads it.
import path from 'node:path';
import { isolated, run, succeed, waitFor, type Ran } from './run';

/**
 * The binary the checks run: `CORTEX_BIN` when set, else the debug build of the repository these
 * checks belong to. The checks never build it.
 */
export const CORTEX =
  process.env.CORTEX_BIN ??
  path.join(
    import.meta.dirname,
    '..',
    '..',
    '..',
    'target',
    'debug',
    process.platform === 'win32' ? 'cortex.exe' : 'cortex',
  );

/** The repository the checks map and the home folder its user has. */
export interface Place {
  /** The repository's root folder. */
  readonly root: string;
  /** The user's home folder. */
  readonly home: string;
}

/** Runs `cortex` with `args` and `--root` in the repository of `place`, from its root. */
export function cortex(place: Place, args: readonly string[], withinMs?: number): Promise<Ran> {
  return run(CORTEX, [...args, '--root', place.root], {
    cwd: place.root,
    env: isolated(place.home),
    ...(withinMs === undefined ? {} : { withinMs }),
  });
}

/** What `cortex` printed with `args` in the repository of `place`, having exited 0. */
export function cortexOut(place: Place, args: readonly string[]): Promise<string> {
  return succeed(CORTEX, [...args, '--root', place.root], {
    cwd: place.root,
    env: isolated(place.home),
  });
}

/**
 * The map's address as `cortex map --no-open` prints it for the repository of `place`, which starts
 * the daemon and the map when they are not running: `http://127.0.0.1:<port>/<token>/`. Rejects
 * when it prints anything else.
 */
export async function mapAddress(place: Place): Promise<string> {
  const printed = await cortexOut(place, ['map', '--no-open']);
  const address = printed.trimEnd();
  if (printed !== `${address}\n` || !/^http:\/\/127\.0\.0\.1:\d+\/[0-9a-f]{64}\/$/.test(address)) {
    throw new Error(`cortex map printed something other than one address: ${printed}`);
  }
  return address;
}

/** Whether `value` is an object, read as one whose fields are not yet known. */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * The JSON body of the map's endpoint `endpoint` (`api/graph`, `api/file/3`) at `address`, or
 * undefined when it answers anything but 200 or not at all: the map closes when idle, and the
 * daemon answers 503 while busy; the waits below ask again.
 */
async function bodyAt(address: string, endpoint: string): Promise<unknown> {
  try {
    const response = await fetch(`${address}${endpoint}`, { signal: AbortSignal.timeout(20_000) });
    if (response.status !== 200) return undefined;
    const body: unknown = await response.json();
    return body;
  } catch {
    // The connection was refused or reset: the map closed while idle. The caller asks again.
    return undefined;
  }
}

/**
 * Waits, at most `withinMs`, until `read` gives a value for the body of the map's endpoint
 * `endpoint`, and resolves with that value. Asks `cortex map --no-open` for the address again
 * whenever the map does not answer, as it closes when idle.
 */
export async function answerWhere<T>(
  place: Place,
  endpoint: string,
  what: string,
  read: (body: unknown) => T | undefined,
  withinMs = 120_000,
): Promise<T> {
  let address = await mapAddress(place);
  return waitFor(
    what,
    async () => {
      const body = await bodyAt(address, endpoint);
      if (body === undefined) {
        address = await mapAddress(place);
        return undefined;
      }
      return read(body);
    },
    withinMs,
  );
}

/** A file of `api/graph`, with the fields the checks read. */
export interface GraphFile {
  readonly id: number;
  readonly path: string;
  readonly touchedMs: number | null;
  readonly suspect: number;
  readonly tokens: number;
}

/** `api/graph` with the fields the checks read: its files, and the kind of each link. */
export interface Graph {
  readonly files: readonly GraphFile[];
  readonly linkKinds: readonly string[];
}

/** The number `value` holds, or a refusal naming `what`. */
function numberOf(value: unknown, what: string): number {
  if (typeof value !== 'number') throw new Error(`api/graph: ${what} is not a number`);
  return value;
}

/** `body` read as `api/graph`, or a refusal naming the field it lacks. */
function graphOf(body: unknown): Graph {
  if (!isRecord(body) || !Array.isArray(body.files) || !Array.isArray(body.links)) {
    throw new Error('api/graph is not a graph');
  }
  const files = body.files.map((file: unknown): GraphFile => {
    if (!isRecord(file) || typeof file.path !== 'string') throw new Error('api/graph: a file');
    const touched = file.touched_ms;
    return {
      id: numberOf(file.id, 'id'),
      path: file.path,
      touchedMs: touched === null ? null : numberOf(touched, 'touched_ms'),
      suspect: numberOf(file.suspect, 'suspect'),
      tokens: numberOf(file.tokens, 'tokens'),
    };
  });
  const linkKinds = body.links.map((link: unknown): string => {
    if (!isRecord(link) || typeof link.kind !== 'string') throw new Error('api/graph: a link');
    return link.kind;
  });
  return { files, linkKinds };
}

/** The file at `filePath` in `map`, or a refusal: every file the checks name is in the map. */
export function fileAt(map: Graph, filePath: string): GraphFile {
  const file = map.files.find((candidate) => candidate.path === filePath);
  if (file === undefined) throw new Error(`the map does not hold ${filePath}`);
  return file;
}

/** Waits, at most `withinMs`, until `condition` holds of the map's graph; resolves with it. */
export function graphWhere(
  place: Place,
  what: string,
  condition: (map: Graph) => boolean,
  withinMs?: number,
): Promise<Graph> {
  return answerWhere(
    place,
    'api/graph',
    what,
    (body) => {
      const map = graphOf(body);
      return condition(map) ? map : undefined;
    },
    withinMs,
  );
}

/**
 * Waits until `condition` holds of the side panel's answer (`api/file/<id>`) for the file at
 * `filePath`; the file's id is the one the map's graph gives it now.
 */
export async function panelWhere(
  place: Place,
  filePath: string,
  what: string,
  condition: (panel: Record<string, unknown>) => boolean,
): Promise<void> {
  const id = fileAt(await graphWhere(place, 'the map', () => true), filePath).id;
  await answerWhere(place, `api/file/${id}`, what, (body) =>
    isRecord(body) && condition(body) ? true : undefined,
  );
}
