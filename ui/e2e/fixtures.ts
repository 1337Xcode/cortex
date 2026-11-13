// Repositories the browser checks are served: a ten-file one with every kind of data (reads,
// recent edits, a suspect fact, links of all three kinds, history, what Cortex told agents, and
// receipts with a holdout comparison), a cold one where nothing has been learned, and a generated
// one of 100,000 files. Each follows the daemon's contract, so the page's real guards accept it.
// Each also has a health (finished reading) and a live state (no session running) unless a test
// gives others. The receipts come in three states: a cold store, 29 tasks a side (not enough to
// compare) and 30 a side with the comparison. The ten-file one also has agents' activity, open and
// resolved warnings, a candidate fact on payment.rs and the symbols of book.rs with the lines work
// orders selected; the others have neither activity nor warnings.

/** A JSON value as the fixtures write them. */
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** One file of a fixture repository, as `api/graph` lists it. */
export interface FixtureFile {
  [key: string]: Json;
  id: number;
  path: string;
  lines: number;
  interest: number;
  tokens: number;
  touched_ms: number | null;
  facts: number;
  suspect: number;
  reads: number;
  commits: number;
  commits_counted: boolean;
}

/** A symbol the fixture's search finds. */
export interface FixtureSymbol {
  readonly file: number;
  readonly name: string;
  readonly line: number;
}

/** Everything a fixture repository answers. */
export interface Fixture {
  readonly graph: { version: 1; files: FixtureFile[]; links: Json[] };
  readonly summary: Json;
  readonly symbols: readonly FixtureSymbol[];
  /** `api/file/<id>` bodies by id; any other file id gets an empty detail. */
  readonly details: ReadonlyMap<number, Json>;
  /** `api/health`. */
  readonly health: Json;
  /** The live state sent when the page opens `api/live`. */
  readonly live: Json;
  /** `api/receipts`. */
  readonly receipts: Json;
  /** `api/activity`. */
  readonly activity: Json;
  /** `api/warnings`. */
  readonly warnings: Json;
}

/** The health of a daemon that has finished reading the repository and cannot say more. */
export const READY_HEALTH: Json = {
  version: '0.0.0',
  has_assets: true,
  index: 'ready',
  daemon: null,
};

/** The health of a daemon still reading the repository for the first time. */
export const READING_HEALTH: Json = {
  version: '0.0.0',
  has_assets: true,
  index: 'reading',
  daemon: null,
};

/**
 * The health of a daemon that has run 2 hours 1 minute: two hooks' handling times, two errors
 * (the newest first), one batch not recorded, the miner done, the graph built, upkeep run and the
 * history worker off.
 */
export function workingHealth(nowMs: number): Json {
  return {
    version: '0.0.0',
    has_assets: true,
    index: 'ready',
    daemon: {
      uptime_ms: 7_260_000,
      hooks: [
        { hook: 'pre_tool', requests: 412, p50_us: 850, p95_us: 2400, p99_us: 9100 },
        { hook: 'prompt', requests: 37, p50_us: 14_200, p95_us: 31_000, p99_us: 48_500 },
      ],
      errors: [
        { at_ms: nowMs - 5 * MINUTE_MS, count: 3, message: 'the watcher lost an event' },
        { at_ms: nowMs - 2 * HOUR_MS, count: 1, message: 'a hook payload was not JSON' },
      ],
      events_dropped: 0,
      failed_batches: 1,
      git: 'ready',
      git_detail: null,
      graph_nodes: 1200,
      graph_links: 5400,
      upkeep_at_ms: nowMs - HOUR_MS,
      history: 'off',
    },
  };
}

/** No event and no warning, as a store no agent has worked in holds. */
export const NO_ACTIVITY: Json = { version: 1, events: [] };
export const NO_WARNINGS: Json = { version: 1, warnings: [] };

/**
 * One file's `api/file` body: no fact, commit, symbol or told entry, the history worker on and no
 * editor link, changed by `fields`; `history_total` and `symbols_total` follow the lists given.
 */
export function fileDetail(id: number, path: string, fields: { [key: string]: Json } = {}): Json {
  const detail: { [key: string]: Json } = {
    version: 1,
    id,
    path,
    facts: [],
    history: [],
    history_reader: 'on',
    symbols: [],
    selected: [],
    told: [],
    editor_url: null,
    ...fields,
  };
  const length = (key: string): number => {
    const list = detail[key];
    return Array.isArray(list) ? list.length : 0;
  };
  if (!('history_total' in fields)) detail['history_total'] = length('history');
  if (!('symbols_total' in fields)) detail['symbols_total'] = length('symbols');
  return detail;
}

/** The live state with no session running. */
export const NO_SESSIONS: Json = { version: 1, sessions: [] };

/** The history limits every fixture's summary states. */
const HISTORY_LIMITS: Json = { max_commits: 5000, max_age_days: 365, max_files_per_commit: 30 };

const MINUTE_MS = 60 * 1000;
const HOUR_MS = 60 * MINUTE_MS;

function file(id: number, path: string, fields: Partial<FixtureFile>): FixtureFile {
  return {
    id,
    path,
    lines: 50,
    interest: 0,
    tokens: 0,
    touched_ms: null,
    facts: 0,
    suspect: 0,
    reads: 0,
    commits: 0,
    commits_counted: true,
    ...fields,
  };
}

/** The ten-file repository. `nowMs` is when the test runs, so "recent" means recent. */
export function smallFixture(nowMs: number): Fixture {
  const files = [
    file(0, 'README.md', { lines: 40, commits: 3 }),
    file(1, 'src/billing/invoice.rs', {
      lines: 300,
      reads: 4,
      tokens: 6200,
      commits: 11,
      interest: 0.4,
    }),
    file(2, 'src/billing/tax.rs', { lines: 120, reads: 2, tokens: 1800, commits: 7 }),
    file(3, 'src/lib.rs', { lines: 60, reads: 6, tokens: 900, commits: 9 }),
    file(4, 'src/orders/book.rs', {
      lines: 500,
      reads: 14,
      tokens: 18300,
      commits: 21,
      facts: 2,
      suspect: 1,
      interest: 0.9,
    }),
    file(5, 'src/orders/mod.rs', { lines: 80, reads: 5, tokens: 4100, commits: 15 }),
    file(6, 'src/orders/payment.rs', {
      lines: 220,
      reads: 9,
      tokens: 29910,
      commits: 13,
      facts: 1,
      touched_ms: nowMs - 2 * HOUR_MS,
    }),
    file(7, 'tests/orders.rs', { lines: 150, commits: 8 }),
    file(8, 'ui/app.ts', {
      lines: 200,
      reads: 3,
      tokens: 2600,
      commits: 5,
      touched_ms: nowMs - 5 * HOUR_MS,
    }),
    file(9, 'ui/view.ts', { lines: 90, commits: 2, touched_ms: nowMs - 72 * HOUR_MS }),
  ];
  // `count` is what each link's weight is made from: a co-change link's commits, the tasks that
  // used both files, the work orders that named both.
  const link = (from: number, to: number, kind: string, weight: number, count: number): Json => ({
    from,
    to,
    kind,
    weight,
    commits: kind === 'co_change' ? count : 0,
    count,
  });
  const links = [
    link(1, 2, 'co_change', 0.7, 5),
    link(4, 5, 'co_change', 0.9, 12),
    link(4, 6, 'co_change', 0.8, 9),
    link(4, 7, 'co_change', 0.55, 6),
    link(6, 7, 'co_change', 0.3, 3),
    link(1, 6, 'used_together', 0.6, 4),
    link(5, 6, 'used_together', 0.4, 2),
    link(8, 9, 'used_together', 0.5, 3),
    link(0, 3, 'work_order', 0.1, 1),
    link(2, 6, 'work_order', 0.35, 2),
    link(4, 8, 'work_order', 0.45, 3),
  ];
  const payment = fileDetail(6, 'src/orders/payment.rs', {
    facts: [
      {
        id: 11,
        kind: 'convention',
        status: 'candidate',
        text: 'A payment is charged once per order.',
      },
    ],
  });
  const book = fileDetail(4, 'src/orders/book.rs', {
    facts: [
      {
        id: 3,
        kind: 'invariant',
        status: 'active',
        text: 'Order totals are kept in integer cents.',
      },
      {
        id: 8,
        kind: 'convention',
        status: 'suspect',
        text: 'A cancelled order keeps its book entry.',
      },
    ],
    history: [
      { subject: 'Keep cancelled orders in the book', refs: '#512', date_s: 1762300000 },
      { subject: 'Charge in integer cents', refs: '#482', date_s: 1761000000 },
    ],
    history_total: 2,
    symbols: [
      {
        name: 'OrderBook',
        kind: 'class',
        start_line: 40,
        end_line: 120,
        is_nested: false,
        selected: 2,
      },
      { name: 'add', kind: 'method', start_line: 52, end_line: 70, is_nested: true, selected: 2 },
      {
        name: 'cancel',
        kind: 'method',
        start_line: 88,
        end_line: 104,
        is_nested: true,
        selected: 0,
      },
      {
        name: 'total_cents',
        kind: 'function',
        start_line: 140,
        end_line: 152,
        is_nested: false,
        selected: 0,
      },
    ],
    selected: [{ start_line: 50, end_line: 72, orders: 2 }],
    told: [
      { kind: 'work_order', at_ms: nowMs - HOUR_MS, used: true },
      { kind: 'warning', at_ms: nowMs - 30 * HOUR_MS, used: null },
    ],
    editor_url: 'vscode://file/C:/repo/src/orders/book.rs:1',
  });
  return {
    graph: { version: 1, files, links },
    summary: {
      version: 1,
      files: 10,
      folders: 5,
      explored_files: 7,
      learned: true,
      strongest_pair: { from: 4, to: 5, commits: 12 },
      busiest_folder: { path: 'src/orders', tokens: 52310 },
      window_days: 7,
      history: HISTORY_LIMITS,
    },
    symbols: [
      { file: 4, name: 'OrderBook', line: 40 },
      { file: 1, name: 'Invoice', line: 12 },
      { file: 6, name: 'charge_order', line: 88 },
    ],
    details: new Map([
      [4, book],
      [6, payment],
    ]),
    health: READY_HEALTH,
    live: NO_SESSIONS,
    receipts: comparedReceipts(),
    activity: smallActivity(nowMs),
    warnings: smallWarnings(nowMs),
  };
}

/** A file as an answer names it: its id in the daemon's graph and its path, or neither. */
export type Named = readonly [number | null, string | null];

/** The ten-file repository's files the activity and warnings name, by id and path. */
const BOOK: Named = [4, 'src/orders/book.rs'];
const MOD: Named = [5, 'src/orders/mod.rs'];
const INVOICE: Named = [1, 'src/billing/invoice.rs'];
const TAX: Named = [2, 'src/billing/tax.rs'];
const LIB: Named = [3, 'src/lib.rs'];
const PAYMENT: Named = [6, 'src/orders/payment.rs'];
const ORDERS_TEST: Named = [7, 'tests/orders.rs'];
const NONE: Named = [null, null];

/** One event of the activity, `agoMs` before `nowMs`, about `file` (or none). */
export function happened(
  nowMs: number,
  agoMs: number,
  kind: string,
  who: { readonly session: string; readonly agent: string | null; readonly title: string | null },
  file: Named,
): Json {
  return { at_ms: nowMs - agoMs, kind, ...who, file: file[0], path: file[1] };
}

/** Claude Code's session 3f2a… and Codex's 9b1c…, as the live fixtures name them. */
export const CLAUDE = { session: '3f2a9c1e', agent: 'claude', title: 'Claude Code' };
const CODEX = { session: '9b1c40aa', agent: 'codex', title: 'Codex' };

/**
 * Seven events, newest first: Claude Code edited book.rs 2 minutes ago, read mod.rs 4 minutes ago,
 * ran a shell command 6 minutes ago; Codex read invoice.rs 9 minutes ago and was given a prompt 10
 * minutes ago; Claude Code read book.rs 12 minutes ago and was given a prompt 13 minutes ago.
 */
export function smallActivity(nowMs: number): Json {
  return {
    version: 1,
    events: [
      happened(nowMs, 2 * MINUTE_MS, 'edit', CLAUDE, BOOK),
      happened(nowMs, 4 * MINUTE_MS, 'read', CLAUDE, MOD),
      happened(nowMs, 6 * MINUTE_MS, 'shell', CLAUDE, NONE),
      happened(nowMs, 9 * MINUTE_MS, 'read', CODEX, INVOICE),
      happened(nowMs, 10 * MINUTE_MS, 'prompt', CODEX, NONE),
      happened(nowMs, 12 * MINUTE_MS, 'read', CLAUDE, BOOK),
      happened(nowMs, 13 * MINUTE_MS, 'prompt', CLAUDE, NONE),
    ],
  };
}

/**
 * Five warnings, newest first: book.rs usually changes with mod.rs (open, 0.62); tests/orders.rs is
 * a test of book.rs (open, 0.90); payment.rs usually changes with book.rs (followed, 0.80);
 * tax.rs calls a symbol of invoice.rs whose signature changed (not followed, 0.60); and a test of
 * lib.rs the map no longer lists (not followed, 0.70).
 */
export function smallWarnings(nowMs: number): Json {
  return {
    version: 1,
    warnings: [
      givenWarning(nowMs - HOUR_MS, 'cochange', [BOOK, MOD], 0.62, 'open'),
      givenWarning(nowMs - 2 * HOUR_MS, 'test', [BOOK, ORDERS_TEST], 0.9, 'open'),
      givenWarning(nowMs - 26 * HOUR_MS, 'cochange', [PAYMENT, BOOK], 0.8, 'followed'),
      givenWarning(nowMs - 30 * HOUR_MS, 'call_site', [INVOICE, TAX], 0.6, 'not_followed'),
      givenWarning(nowMs - 50 * HOUR_MS, 'test', [LIB, NONE], 0.7, 'not_followed'),
    ],
  };
}

/** One warning as `api/warnings` lists it, about `files[0]` and naming `files[1]`. */
export function givenWarning(
  atMs: number,
  kind: string,
  files: readonly [Named, Named],
  strength: number,
  state: string,
): Json {
  const [source, target] = files;
  return {
    at_ms: atMs,
    kind,
    source: source[0],
    target: target[0],
    source_path: source[1],
    target_path: target[1],
    strength,
    state,
  };
}

/** A repository Cortex has learned nothing about: structure only. */
export function coldFixture(): Fixture {
  const paths = [
    'Cargo.toml',
    'src/cli.rs',
    'src/main.rs',
    'src/parse/lexer.rs',
    'src/parse/mod.rs',
    'tests/cli.rs',
  ];
  return {
    graph: {
      version: 1,
      files: paths.map((path, id) => file(id, path, { lines: 40 + id * 30 })),
      links: [],
    },
    summary: {
      version: 1,
      files: 6,
      folders: 3,
      explored_files: 0,
      learned: false,
      strongest_pair: null,
      busiest_folder: null,
      window_days: 7,
      history: HISTORY_LIMITS,
    },
    symbols: [],
    details: new Map(),
    health: READY_HEALTH,
    live: NO_SESSIONS,
    receipts: coldReceipts(),
    activity: NO_ACTIVITY,
    warnings: NO_WARNINGS,
  };
}

/** A seeded generator (mulberry32), so the large repository is the same on every run. */
function seeded(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Links between nearby ids (files of one folder sit together in path order), one kind at a time. */
function generatedLinks(count: number, random: () => number): Json[] {
  const links: Json[] = [];
  for (const kind of ['co_change', 'used_together', 'work_order']) {
    const share = kind === 'co_change' ? 0.3 : 0.12;
    for (let from = 0; from < count - 1; from += 1) {
      if (random() >= share) continue;
      const to = Math.min(count - 1, from + 1 + Math.floor(random() * 40));
      const weight = Math.round(random() * 100) / 100;
      const commits = kind === 'co_change' ? 1 + Math.floor(random() * 20) : 0;
      // A co-change link is made of its commits; one task or work order for the other kinds, so
      // the generator draws no more numbers than it did before links had counts.
      links.push({ from, to, kind, weight, commits, count: kind === 'co_change' ? commits : 1 });
    }
  }
  return links;
}

/** The number of distinct folders the paths sit in (every proper prefix ending before a `/`). */
function folderCount(paths: readonly string[]): number {
  const folders = new Set<string>();
  for (const path of paths) {
    for (let slash = path.indexOf('/'); slash >= 0; slash = path.indexOf('/', slash + 1)) {
      folders.add(path.slice(0, slash));
    }
  }
  return folders.size;
}

/**
 * `count` paths shaped like a large codebase: a few top folders, nested two to five deep, each
 * folder holding a handful to a few dozen files; a few files at the root. Sorted in byte order
 * (the names are ASCII, so code-unit order is byte order).
 */
function generatedPaths(count: number, random: () => number): string[] {
  const paths = new Set<string>(['README.md', 'Cargo.toml', 'package.json']);
  let folder: string[] = [];
  while (paths.size < count) {
    if (folder.length === 0 || random() < 0.12) {
      const depth = 2 + Math.floor(random() * 4);
      folder = Array.from(
        { length: depth },
        (_, level) =>
          `${level === 0 ? 'pkg' : 'mod'}${Math.floor(random() * (level === 0 ? 12 : 9))}`,
      );
    }
    const width = 3 + Math.floor(random() * 30);
    for (let i = 0; i < width && paths.size < count; i += 1) {
      paths.add(`${folder.join('/')}/file${Math.floor(random() * 400)}.rs`);
    }
    folder = random() < 0.5 ? folder.slice(0, -1) : [];
  }
  return [...paths].sort();
}

/** A generated repository of `count` files with links of every kind. */
export function largeFixture(count: number): Fixture {
  const random = seeded(42);
  const files = generatedPaths(count, random).map((path, id) =>
    file(id, path, {
      lines: 10 + Math.floor(random() * 900),
      reads: random() < 0.3 ? Math.floor(random() * 20) : 0,
      tokens: Math.floor(random() * 5000),
      commits: Math.floor(random() * 30),
    }),
  );
  return {
    graph: { version: 1, files, links: generatedLinks(count, random) },
    summary: {
      version: 1,
      files: count,
      folders: folderCount(files.map((entry) => entry.path)),
      explored_files: files.filter((entry) => entry.reads > 0).length,
      learned: true,
      strongest_pair: null,
      busiest_folder: null,
      window_days: 7,
      history: HISTORY_LIMITS,
    },
    symbols: [],
    details: new Map(),
    health: READY_HEALTH,
    live: NO_SESSIONS,
    receipts: shortReceipts(),
    activity: NO_ACTIVITY,
    warnings: NO_WARNINGS,
  };
}

/** One live session as `api/live` lists it; `files` are (id, mode) pairs, newest first. */
export function liveSession(
  id: string,
  agent: { readonly name: string; readonly title: string },
  files: readonly (readonly [number, string])[],
  nowMs: number,
): Json {
  return {
    id,
    agent: agent.name,
    title: agent.title,
    files: files.map(([file, mode], index) => ({ file, mode, last_ms: nowMs - index * 60_000 })),
  };
}

/** The agents the live fixtures use. */
export const AGENTS = {
  claude: { name: 'claude', title: 'Claude Code' },
  codex: { name: 'codex', title: 'Codex' },
  cursor: { name: 'cursor', title: 'Cursor' },
  other: { name: 'aider', title: 'Aider' },
} as const;

/**
 * Three sessions on the ten-file repository: Claude Code editing book.rs after reading mod.rs,
 * payment.rs and tax.rs; Codex reading invoice.rs after app.ts; Cursor claiming tests/orders.rs.
 * Sorted by id, as the daemon sends them.
 */
export function threeSessions(nowMs: number): Json {
  return {
    version: 1,
    sessions: [
      liveSession(
        '3f2a9c1e',
        AGENTS.claude,
        [
          [4, 'edit'],
          [5, 'read'],
          [6, 'read'],
          [2, 'read'],
        ],
        nowMs,
      ),
      liveSession(
        '9b1c40aa',
        AGENTS.codex,
        [
          [1, 'read'],
          [8, 'read'],
        ],
        nowMs,
      ),
      liveSession('c0de7731', AGENTS.cursor, [[7, 'claim']], nowMs),
    ],
  };
}

/** `count` sessions spread over a repository of `fileCount` files, each with ten files. */
export function manySessions(count: number, fileCount: number, nowMs: number): Json {
  const agents = [AGENTS.claude, AGENTS.codex, AGENTS.cursor, AGENTS.other];
  const sessions: Json[] = [];
  for (let index = 0; index < count; index += 1) {
    const start = Math.floor(((index + 0.5) * fileCount) / count);
    const files: [number, string][] = Array.from({ length: 10 }, (_, step) => [
      Math.min(fileCount - 1, start + step * 7),
      step === 0 ? 'edit' : 'read',
    ]);
    const id = `s${String(index).padStart(3, '0')}-session`;
    sessions.push(liveSession(id, agents[index % agents.length] ?? AGENTS.other, files, nowMs));
  }
  return { version: 1, sessions };
}

/** 2026-11-10 09:00 UTC: the receipts' sessions start from here, so their times read the same. */
export const RECEIPTS_BASE_MS = Date.UTC(2026, 10, 10, 9, 0);

const DAY_MS = 24 * HOUR_MS;

/** What each response costs in the receipt fixtures: $0.0025, $0.0005 of it on cache writes. */
const PICODOLLARS_PER_TURN = 2_500_000_000;
const CACHE_WRITE_PICODOLLARS_PER_TURN = 500_000_000;

/** Input tokens by part: uncached, read from the cache, written to it. */
type InputParts = readonly [number, number, number];

/** One task of a receipt fixture. */
interface ReceiptTask {
  readonly seq: number;
  readonly startMs: number;
  readonly turns: number;
  readonly input: InputParts;
  readonly output: number;
  readonly calls: number;
  readonly injected: number;
  readonly outcome: string | null;
  readonly heldOut: boolean | null;
}

/** What a session did outside its tasks. */
interface Outside {
  readonly turns: number;
  readonly input: InputParts;
  readonly output: number;
  readonly calls: number;
}

/** A tally of `turns` responses, every one priced, its input the sum of its three parts. */
function receiptTally(turns: number, input: InputParts, output: number): Json {
  const [uncached, reads, writes] = input;
  return {
    turns,
    cumulative_input: uncached + reads + writes,
    uncached_input: uncached,
    cache_reads: reads,
    cache_writes: writes,
    output,
    estimate: {
      picodollars: PICODOLLARS_PER_TURN * turns,
      cache_write_picodollars: CACHE_WRITE_PICODOLLARS_PER_TURN * turns,
      priced_turns: turns,
      unpriced_turns: {},
    },
  };
}

/** A task's receipt as the daemon writes it; a task with an outcome ended 20 minutes in. */
function receiptTask(task: ReceiptTask): Json {
  return {
    task_id: `01JCRECE1PT00000000000${String(task.seq).padStart(4, '0')}`,
    seq: task.seq,
    started_at_ms: task.startMs,
    ended_at_ms: task.outcome === null ? null : task.startMs + 20 * MINUTE_MS,
    outcome: task.outcome,
    held_out: task.heldOut,
    tool_calls: task.calls,
    injected_tokens: task.injected,
    usage: receiptTally(task.turns, task.input, task.output),
  };
}

/** A session's receipt: its tasks, what fell outside them, and the sums of both. */
function receiptSession(
  head: { id: string; agent: string; isMetered: boolean; startMs: number; status: string },
  tasks: readonly ReceiptTask[],
  outside: Outside,
  primer: number,
): Json {
  const sum = (count: (task: ReceiptTask) => number): number =>
    tasks.reduce((total, task) => total + count(task), 0);
  const input: InputParts = [
    sum((task) => task.input[0]) + outside.input[0],
    sum((task) => task.input[1]) + outside.input[1],
    sum((task) => task.input[2]) + outside.input[2],
  ];
  const turns = sum((task) => task.turns) + outside.turns;
  return {
    session_id: head.id,
    agent: head.agent,
    is_metered: head.isMetered,
    started_at_ms: head.startMs,
    ended_at_ms: head.status === 'live' ? null : head.startMs + 2 * HOUR_MS,
    status: head.status,
    subagents: 0,
    tool_calls: sum((task) => task.calls) + outside.calls,
    tasks: tasks.map(receiptTask),
    outside_tasks: receiptTally(outside.turns, outside.input, outside.output),
    outside_tool_calls: outside.calls,
    total: receiptTally(turns, input, sum((task) => task.output) + outside.output),
    compactions_left_out: 0,
    cache_breaks: [],
    primer_tokens: primer,
    injected_tokens: primer + sum((task) => task.injected),
  };
}

/** A task with no usage, of an agent whose transcripts Cortex does not read. */
function unreadTask(
  seq: number,
  startMs: number,
  fields: Pick<ReceiptTask, 'calls' | 'injected' | 'outcome' | 'heldOut'>,
): ReceiptTask {
  return { seq, startMs, turns: 0, input: [0, 0, 0], output: 0, ...fields };
}

/**
 * Three sessions of the window, oldest first:
 * - Claude Code (ended): task 101 (6 turns, 2,000 + 30,000 + 4,000 = 36,000 input tokens) and
 *   task 102 (held out; 3 turns, 500 + 12,000 + 500 = 13,000), and 1 turn of 300 tokens outside
 *   them: 49,300 input tokens in all; 400 (primer) + 350 = 750 tokens injected.
 * - Codex (ended), whose usage Cortex does not read: tasks 103 and 104; 120 tokens injected.
 * - Claude Code (running): task 105 (4 turns, 1,000 + 20,000 + 1,000 = 22,000) and task 106
 *   (open; 2 turns, 400 + 9,000 + 600 = 10,000): 32,000 input tokens; 300 + 210 + 90 = 600 injected.
 * The chart draws tasks 101, 102, 105 and 106.
 */
export function receiptSessions(): Json[] {
  const base = RECEIPTS_BASE_MS;
  const none: Outside = { turns: 0, input: [0, 0, 0], output: 0, calls: 0 };
  const success = { outcome: 'success', heldOut: false };
  return [
    receiptSession(
      { id: 'a1b2c3d4-0001', agent: 'claude', isMetered: true, startMs: base, status: 'ended' },
      [
        {
          seq: 101,
          startMs: base + 5 * MINUTE_MS,
          turns: 6,
          input: [2000, 30_000, 4000],
          output: 1200,
          calls: 9,
          injected: 350,
          ...success,
        },
        {
          seq: 102,
          startMs: base + 40 * MINUTE_MS,
          turns: 3,
          input: [500, 12_000, 500],
          output: 400,
          calls: 2,
          injected: 0,
          outcome: 'failure',
          heldOut: true,
        },
      ],
      { turns: 1, input: [300, 0, 0], output: 50, calls: 1 },
      400,
    ),
    receiptSession(
      {
        id: 'e5f6a7b8-0002',
        agent: 'codex',
        isMetered: false,
        startMs: base + DAY_MS,
        status: 'ended',
      },
      [
        unreadTask(103, base + DAY_MS + 10 * MINUTE_MS, {
          calls: 5,
          injected: 120,
          outcome: 'unknown',
          heldOut: false,
        }),
        unreadTask(104, base + DAY_MS + 30 * MINUTE_MS, {
          calls: 3,
          injected: 0,
          outcome: 'success',
          heldOut: true,
        }),
      ],
      none,
      0,
    ),
    receiptSession(
      {
        id: 'c9d0e1f2-0003',
        agent: 'claude',
        isMetered: true,
        startMs: base + 2 * DAY_MS,
        status: 'live',
      },
      [
        {
          seq: 105,
          startMs: base + 2 * DAY_MS + 2 * MINUTE_MS,
          turns: 4,
          input: [1000, 20_000, 1000],
          output: 600,
          calls: 6,
          injected: 210,
          ...success,
        },
        {
          seq: 106,
          startMs: base + 2 * DAY_MS + 50 * MINUTE_MS,
          turns: 2,
          input: [400, 9000, 600],
          output: 200,
          calls: 1,
          injected: 90,
          outcome: null,
          heldOut: null,
        },
      ],
      none,
      300,
    ),
  ];
}

/** A store with nothing recorded and no price table, as on a repository Cortex has just met. */
export function coldReceipts(): Json {
  return {
    version: 1,
    window_days: 7,
    prices: { state: 'missing', why: 'there is no home folder to keep a price table in' },
    recorded: {
      holdout: { held_out: 0, treated: 0, needed_per_arm: 30, comparison: null },
      precision: { labelled: 0, used: 0 },
      warnings: { labelled: 0, used: 0 },
      sessions: [],
    },
  };
}

/** 29 tasks a side, one short of a comparison, priced from the shipped table. */
export function shortReceipts(): Json {
  return {
    version: 1,
    window_days: 7,
    prices: { state: 'shipped', updated: '2026-11-01' },
    recorded: {
      holdout: { held_out: 29, treated: 29, needed_per_arm: 30, comparison: null },
      precision: { labelled: 9, used: 7 },
      warnings: { labelled: 4, used: 1 },
      sessions: receiptSessions(),
    },
  };
}

/** One measure compared, the difference with Cortex minus held out as the daemon takes it. */
function compared(treated: number, heldOut: number, low: number, high: number): Json {
  return { treated, held_out: heldOut, difference: treated - heldOut, low, high };
}

/**
 * 30 tasks a side with the comparison, priced from the user's file: success 0.8 with Cortex (24 of
 * 30) and 0.7 held out (21 of 30), and the four medians per successful task; every interval but
 * the input tokens' holds 0.
 */
export function comparedReceipts(): Json {
  return {
    version: 1,
    window_days: 7,
    prices: { state: 'read', updated: '2026-11-01' },
    recorded: {
      holdout: { held_out: 30, treated: 30, needed_per_arm: 30, comparison: holdoutComparison() },
      precision: { labelled: 73, used: 52 },
      warnings: { labelled: 9, used: 5 },
      sessions: receiptSessions(),
    },
  };
}

/** The comparison of `comparedReceipts`, on its own. */
function holdoutComparison(): Json {
  return {
    tasks: [30, 30],
    success_rate: compared(0.8, 0.7, -0.1, 0.3),
    per_success_tasks: [30, 30],
    per_success: [
      compared(21_000, 34_000, -20_000, -5000),
      compared(900, 1500, -1100, 100),
      compared(4, 6, -3, 0),
      compared(5.5, 7, -3, 0.5),
    ],
  };
}

/**
 * 29 tasks a side carrying a comparison anyway, which the daemon never sends: the page must refuse
 * the answer rather than show a comparison resting on too few tasks.
 */
export function shortReceiptsWithComparison(): Json {
  return {
    version: 1,
    window_days: 7,
    prices: { state: 'shipped', updated: '2026-11-01' },
    recorded: {
      holdout: { held_out: 29, treated: 29, needed_per_arm: 30, comparison: holdoutComparison() },
      precision: { labelled: 9, used: 7 },
      warnings: { labelled: 4, used: 1 },
      sessions: receiptSessions(),
    },
  };
}
