// The scripted repository the real-daemon checks map: six files in three folders, committed by a
// fixed author at fixed times in ten commits chosen so that every answer the map gives about its
// history is known before the daemon reads it. Each file's text is written here line by line, and
// its line count is counted by hand in the comment above it.
import fs from 'node:fs';
import path from 'node:path';
import { isolated, succeed } from './run';

/** A file of the repository: its path from the root, with `/`, and its lines. */
export interface Source {
  readonly path: string;
  readonly lines: readonly string[];
}

/** The text of `source` as it is written on disk: its lines, each ended by `\n`. */
export function textOf(source: Source): string {
  return source.lines.map((line) => `${line}\n`).join('');
}

/** `main.py`: 3 lines, committed once. */
export const MAIN: Source = {
  path: 'main.py',
  lines: ['from calc.ops import add', '', 'print(add(1, 2))'],
};

/** The first `calc/ops.py`: 10 lines; `divide`, on lines 9 and 10, divides by floor. */
const OPS_1: Source = {
  path: 'calc/ops.py',
  lines: [
    'def add(a, b):',
    '    return a + b',
    '',
    '',
    'def mul(a, b):',
    '    return a * b',
    '',
    '',
    'def divide(a, b):',
    '    return a // b',
  ],
};

/** `calc/ops.py` with line 10 of {@link OPS_1} replaced by `line`. */
function opsDividing(line: string): Source {
  return { path: OPS_1.path, lines: [...OPS_1.lines.slice(0, 9), line] };
}

/** The second `calc/ops.py`: `divide` divides truly. */
const OPS_2 = opsDividing('    return a / b');

/** The third `calc/ops.py`: `divide` rounds to cents. */
const OPS_3 = opsDividing('    return round(a / b, 2)');

/**
 * `calc/ops.py` as committed last: the third with a docstring and two blank lines in front, 13
 * lines. `mul`'s body is line 9 and `divide`'s line 13.
 */
export const OPS: Source = {
  path: OPS_1.path,
  lines: ['"""Arithmetic helpers."""', '', '', ...OPS_3.lines],
};

/** The body of `mul` as committed, which the agent's edit turns into {@link MUL_EDITED}. */
export const MUL_BODY = '    return a * b';

/** The body of `mul` once the agent edited it: the same product, its factors swapped. */
export const MUL_EDITED = '    return b * a';

/** `calc/ops.py` once the agent's edit swapped `mul`'s factors: still 13 lines. */
export const OPS_EDITED: Source = {
  path: OPS.path,
  lines: OPS.lines.map((line) => (line === MUL_BODY ? MUL_EDITED : line)),
};

/** `calc/ops.py` once someone changed `divide` outside any agent, after the fact about it was noted. */
export const OPS_CHANGED: Source = {
  path: OPS.path,
  lines: OPS_EDITED.lines.map((line) =>
    line === '    return round(a / b, 2)' ? '    return round(a / b, 4)' : line,
  ),
};

/** `docs/ops.md` with its third line `line`: 3 lines. */
function docsSaying(line: string): Source {
  return { path: 'docs/ops.md', lines: ['# Arithmetic helpers', '', line] };
}

/** `docs/ops.md` as committed last. */
export const DOCS = docsSaying('`divide` rounds to cents.');

/** The first `calc/money.py`: 6 lines, the class `Ledger`. */
const MONEY_1: Source = {
  path: 'calc/money.py',
  lines: [
    '"""A ledger of amounts."""',
    '',
    '',
    'class Ledger:',
    '    def __init__(self):',
    '        self.amounts = []',
  ],
};

/** The second `calc/money.py`: the first and 3 lines more, `total`: 9 lines. */
const MONEY_2: Source = {
  path: MONEY_1.path,
  lines: [...MONEY_1.lines, '', '    def total(self):', '        return sum(self.amounts)'],
};

/** `calc/money.py` as committed last: the second and 3 lines more, `add_cents`: 12 lines. */
export const MONEY: Source = {
  path: MONEY_1.path,
  lines: [
    ...MONEY_2.lines,
    '',
    '    def add_cents(self, cents):',
    '        self.amounts.append(int(cents))',
  ],
};

/** `calc/rates.py`: 5 lines, committed once. */
export const RATES: Source = {
  path: 'calc/rates.py',
  lines: [
    'RATES = {"EUR": 1.08, "GBP": 1.27}',
    '',
    '',
    'def rate(code):',
    '    return RATES[code]',
  ],
};

/** `tests/test_ops.py`: 14 lines, the tests of `calc/ops.py` by its name, committed once. */
export const TESTS: Source = {
  path: 'tests/test_ops.py',
  lines: [
    'import unittest',
    '',
    'from calc.ops import add, divide, mul',
    '',
    '',
    'class OpsTest(unittest.TestCase):',
    '    def test_add(self):',
    '        self.assertEqual(add(2, 3), 5)',
    '',
    '    def test_mul(self):',
    '        self.assertEqual(mul(2, 3), 6)',
    '',
    '    def test_divide(self):',
    '        self.assertEqual(divide(7, 2), 3.5)',
  ],
};

/** One commit: its subject and the files it writes, in the state it leaves them. */
interface Commit {
  readonly subject: string;
  readonly files: readonly Source[];
}

/**
 * The repository's history, oldest first, one commit every ten minutes from 09:00 UTC on
 * 2026-11-01. Counted by hand:
 *
 * - `calc/ops.py` changes in 4 commits (2, 5, 8, 10) and `docs/ops.md` in 3 (2, 5, 8), always with
 *   `calc/ops.py`: they changed together in 3 commits, 3 of `calc/ops.py`'s 4, and no other two
 *   files ever changed in one commit. The map's only co-change link is theirs, with 3 commits.
 * - `calc/money.py` changes in 3 commits (3, 6, 9), each adding lines the next keeps, so the
 *   commits behind its 12 lines are all three: lines 1 to 6 are commit 3's, 7 to 9 commit 6's and
 *   10 to 12 commit 9's.
 * - `main.py`, `tests/test_ops.py` and `calc/rates.py` change once each, alone.
 * - No subject names an issue, and the only commit that touched a test (4) changed nothing else.
 */
export const HISTORY: readonly Commit[] = [
  { subject: 'Add the entry point', files: [MAIN] },
  { subject: 'Add arithmetic helpers', files: [OPS_1, docsSaying('`divide` divides by floor.')] },
  { subject: 'Add the ledger', files: [MONEY_1] },
  { subject: 'Test the helpers', files: [TESTS] },
  { subject: 'Divide truly', files: [OPS_2, docsSaying('`divide` divides truly.')] },
  { subject: 'Total the ledger', files: [MONEY_2] },
  { subject: 'Add exchange rates', files: [RATES] },
  { subject: 'Round division to cents', files: [OPS_3, DOCS] },
  { subject: 'Keep ledger amounts in cents', files: [MONEY] },
  { subject: 'Name the helpers in a docstring', files: [OPS] },
];

/** When the first commit was made, in UTC milliseconds: 2026-11-01 09:00:00. */
const FIRST_COMMIT_MS = Date.UTC(2026, 10, 1, 9, 0, 0);

/** The time between two commits: ten minutes. */
const COMMIT_EVERY_MS = 10 * 60 * 1000;

/**
 * The repository's settings: medium mode, so that every warning rule runs whatever the repository's
 * size; no task held out, so that the agent's one task is a treated one; and the history worker on
 * (`history.why`), which reads blames with the local `git` alone, so that a file's panel can name
 * the commits behind its lines.
 */
export const CONFIG = 'mode = "medium"\n\n[holdout]\nshare = 0\n\n[history]\nwhy = true\n';

/** Writes `source` under `root`, making its folder. */
export function writeSource(root: string, source: Source): void {
  const file = path.join(root, ...source.path.split('/'));
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, textOf(source));
}

/**
 * Makes the repository at `root`, whose user's home folder is `home`: git initialised, each commit
 * of {@link HISTORY} made in turn by a fixed author at its time, then `.cortex/config.toml` written
 * with {@link CONFIG}, which git is not asked to commit.
 */
export async function makeRepository(root: string, home: string): Promise<void> {
  fs.mkdirSync(root, { recursive: true });
  const env = isolated(home);
  const git = (args: readonly string[], extra: Record<string, string> = {}): Promise<string> =>
    succeed('git', args, { cwd: root, env: { ...env, ...extra } });
  await git(['init', '-q', '-b', 'main']);
  for (const [index, commit] of HISTORY.entries()) {
    for (const file of commit.files) writeSource(root, file);
    await git(['add', '--', ...commit.files.map((file) => file.path)]);
    const at = new Date(FIRST_COMMIT_MS + index * COMMIT_EVERY_MS).toISOString();
    await git(['commit', '-q', '-m', commit.subject], {
      GIT_AUTHOR_NAME: 'Cortex Test',
      GIT_AUTHOR_EMAIL: 'test',
      GIT_AUTHOR_DATE: at,
      GIT_COMMITTER_NAME: 'Cortex Test',
      GIT_COMMITTER_EMAIL: 'test',
      GIT_COMMITTER_DATE: at,
    });
  }
  fs.mkdirSync(path.join(root, '.cortex'), { recursive: true });
  fs.writeFileSync(path.join(root, '.cortex', 'config.toml'), CONFIG);
}
