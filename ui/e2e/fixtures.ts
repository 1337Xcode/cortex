// Repositories the browser checks are served: a ten-file one with every kind of data (reads,
// recent edits, a suspect fact, links of all three kinds, history, what Cortex told agents), a
// cold one where nothing has been learned, and a generated one of 100,000 files. Each follows the
// daemon's contract, so the page's real guards accept it.

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
}

const HOUR_MS = 60 * 60 * 1000;

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
  const link = (from: number, to: number, kind: string, weight: number, commits = 0): Json => ({
    from,
    to,
    kind,
    weight,
    commits,
  });
  const links = [
    link(1, 2, 'co_change', 0.7, 5),
    link(4, 5, 'co_change', 0.9, 12),
    link(4, 6, 'co_change', 0.8, 9),
    link(4, 7, 'co_change', 0.55, 6),
    link(6, 7, 'co_change', 0.3, 3),
    link(1, 6, 'used_together', 0.6),
    link(5, 6, 'used_together', 0.4),
    link(8, 9, 'used_together', 0.5),
    link(0, 3, 'work_order', 0.1),
    link(2, 6, 'work_order', 0.35),
    link(4, 8, 'work_order', 0.45),
  ];
  const book = {
    version: 1,
    id: 4,
    path: 'src/orders/book.rs',
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
    told: [
      { kind: 'work_order', at_ms: nowMs - HOUR_MS, used: true },
      { kind: 'warning', at_ms: nowMs - 30 * HOUR_MS, used: null },
    ],
  };
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
    },
    symbols: [
      { file: 4, name: 'OrderBook', line: 40 },
      { file: 1, name: 'Invoice', line: 12 },
      { file: 6, name: 'charge_order', line: 88 },
    ],
    details: new Map([[4, book]]),
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
    },
    symbols: [],
    details: new Map(),
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
      links.push({ from, to, kind, weight, commits });
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
    },
    symbols: [],
    details: new Map(),
  };
}
