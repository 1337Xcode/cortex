// Tests for the words of the overview's last panels and the file panel's new parts: the activity,
// the warnings, the daemon's health, the commits behind a file's lines, the person's word on a
// fact and the file's symbols. Every expected sentence is written out.
import { describe, expect, it } from 'vitest';
import type { ActivityEvent } from '../api/activity';
import type { FileSymbol } from '../api/file';
import type { DaemonHealth } from '../api/health';
import type { GivenWarning } from '../api/warnings';
import { eventLine, eventSentence, shortAgo } from './activityWords';
import { outcomeSentence, retireQuestion, wordLabel, wordsFor } from './factWords';
import {
  errorSentence,
  gitSentence,
  graphSentence,
  HOOK_TIMES_NOTE,
  hookName,
  runSentence,
} from './healthWords';
import { HISTORY_TITLE, historyNote } from './historyWords';
import { spanSentence, symbolSentence, symbolsNote } from './symbolWords';
import { openFirst, strengthText, warningMeta, warningSentence } from './warningWords';

// 1762900000000 ms is 2025-11-11 22:26:40 UTC.
const AT_MS = 1762900000000;

const EDIT: ActivityEvent = {
  at_ms: AT_MS,
  kind: 'edit',
  session: '3f2a9c1e',
  agent: { name: 'claude', title: 'Claude Code' },
  file: 4,
  path: 'src/orders/book.rs',
};

describe('the activity', () => {
  it('says each event as a sentence, short id and all', () => {
    expect(eventLine(EDIT, 'book.rs')).toBe('Claude Code 3f2a… edited book.rs');
    expect(eventLine({ ...EDIT, kind: 'read' }, 'mod.rs')).toBe('Claude Code 3f2a… read mod.rs');
    expect(eventLine({ ...EDIT, kind: 'shell' }, 'tax.rs')).toBe(
      'Claude Code 3f2a… ran a shell command, about tax.rs',
    );
    expect(eventLine({ ...EDIT, kind: 'prompt', file: null, agent: null }, null)).toBe(
      'Session 3f2a… was given a prompt',
    );
    expect(eventLine({ ...EDIT, file: 900 }, null)).toBe(
      'Claude Code 3f2a… edited a file the map does not list',
    );
  });

  it('counts back from when the list was read, and gives the exact time in UTC', () => {
    // Read 2 minutes 30 seconds after the edit.
    expect(shortAgo(AT_MS, AT_MS + 150_000)).toBe('2 min ago');
    expect(
      [0, 999, 59_000, 3 * 3_600_000, 2 * 86_400_000].map((d) => shortAgo(AT_MS, AT_MS + d)),
    ).toEqual(['just now', 'just now', '59 s ago', '3 h ago', '2 d ago']);
    expect(shortAgo(AT_MS + 5000, AT_MS)).toBe('later');
    expect(eventSentence(EDIT, 'book.rs', AT_MS + 150_000)).toBe(
      'Claude Code 3f2a… edited book.rs, 2 min ago, at 2025-11-11 22:26:40 UTC.',
    );
  });
});

describe('the warnings', () => {
  const given: GivenWarning = {
    at_ms: AT_MS,
    kind: 'cochange',
    source: 4,
    target: 5,
    source_path: 'src/orders/book.rs',
    target_path: 'src/orders/mod.rs',
    strength: 0.62,
    state: 'followed',
  };

  it('say each kind in words, with what its strength measures and what became of it', () => {
    expect(warningSentence(given, 'book.rs', 'mod.rs')).toBe(
      'book.rs usually changes with mod.rs. Given 2025-11-11; followed: the named file was read or edited after it. Strength 0.62 of 1: how often the two files changed, or were worked on, together.',
    );
    expect(
      warningSentence(
        { ...given, kind: 'test', state: 'open', strength: 1 },
        'book.rs',
        'orders.rs',
      ),
    ).toBe(
      'orders.rs is a test of book.rs. Given 2025-11-11; open: its task has not ended yet. Strength 1 of 1: how sure Cortex is that a test file tests the edited one, the strongest of the files the warning named.',
    );
    expect(
      warningSentence(
        { ...given, kind: 'call_site', state: 'not_followed', strength: 0.3 },
        'book.rs',
        null,
      ),
    ).toBe(
      'a file the map no longer lists calls a symbol of book.rs whose signature changed. Given 2025-11-11; not followed: its task ended without reading or editing the named file. Strength 0.30 of 1: how firmly the index ties a call to the changed symbol, the strongest of the files the warning named.',
    );
    expect(warningMeta(given)).toEqual(['Followed', 'strength 0.62', '2025-11-11']);
  });

  it('never write a strength that is neither end as 0 or 1', () => {
    expect([0, 0.004, 0.005, 0.62, 0.994, 0.996, 1].map(strengthText)).toEqual([
      '0',
      'under 0.01',
      '0.01',
      '0.62',
      '0.99',
      'over 0.99',
      '1',
    ]);
    expect(warningMeta({ ...given, strength: 0.004 })).toEqual([
      'Followed',
      'strength under 0.01',
      '2025-11-11',
    ]);
  });

  it('list the open first, each part in the daemon order', () => {
    const list = [
      { ...given, at_ms: 3 },
      { ...given, at_ms: 2, state: 'open' as const },
      { ...given, at_ms: 1, state: 'not_followed' as const },
      { ...given, at_ms: 0, state: 'open' as const },
    ];
    expect(openFirst(list).map((warning) => warning.at_ms)).toEqual([2, 0, 3, 1]);
  });
});

describe("the daemon's health", () => {
  const daemon: DaemonHealth = {
    uptime_ms: 7_260_000,
    hooks: [],
    errors: [],
    events_dropped: 0,
    failed_batches: 1,
    git: 'failed',
    git_detail: 'git log exited 128',
    graph: { nodes: 1200, links: 5400 },
    upkeep_at_ms: AT_MS,
    history: 'off',
  };

  it('says each part in a sentence with its numbers', () => {
    expect(runSentence(daemon)).toBe(
      'Running for 2 hours; since it started, 0 hook events dropped and 1 batch of events not recorded.',
    );
    expect(gitSentence(daemon)).toBe(
      'The history miner failed its last reading; it tries again after a pause, or when HEAD moves. git log exited 128',
    );
    expect(graphSentence(daemon)).toBe(
      'The graph holds 1,200 nodes and 5,400 links, as last built; upkeep last ran at 2025-11-11 22:26:40 UTC.',
    );
    expect(graphSentence({ ...daemon, graph: null, upkeep_at_ms: null })).toBe(
      'The graph is not built yet; upkeep has not run yet.',
    );
    expect(errorSentence({ message: 'the watcher lost an event', count: 3, at_ms: AT_MS })).toBe(
      'the watcher lost an event (3 times, last at 2025-11-11 22:26:40 UTC)',
    );
  });

  it("says the hook times are the daemon's own handling, never a budget", () => {
    expect(HOOK_TIMES_NOTE).toContain('not the hook process');
    expect(HOOK_TIMES_NOTE).not.toMatch(/budget/i);
    expect([hookName('pre_tool'), hookName('pre_compact_v2')]).toEqual([
      'Before a tool',
      'pre_compact_v2',
    ]);
  });
});

describe('the commits behind a file’s lines', () => {
  it('are titled so they are not read as the Commits figure', () => {
    expect(HISTORY_TITLE).toBe('Commits behind its lines');
  });

  it('say, with the history worker off, that no file has them and how to turn it on', () => {
    expect(historyNote({ history: [], history_total: 0, history_reader: 'off' })).toBe(
      "Cortex reads the commits behind a file's lines only when history.why or history.seeds is on, and both are off, so no file has them. To read them, run cortex config set history.why true.",
    );
    expect(historyNote({ history: [], history_total: 0, history_reader: 'refused' })).toBe(
      "Git is refused in this repository, so Cortex reads no commits behind any file's lines.",
    );
  });

  it('say the 10 listed are the newest of 12', () => {
    const ten = Array.from({ length: 10 }, () => ({ subject: 's', refs: '', date_s: 1 }));
    expect(historyNote({ history: ten, history_total: 12, history_reader: 'on' })).toBe(
      'The 10 newest of 12 commits behind its current lines.',
    );
    expect(historyNote({ history: ten.slice(0, 2), history_total: 2, history_reader: 'on' })).toBe(
      'The 2 commits behind its current lines, newest first.',
    );
  });
});

describe("the person's word on a fact", () => {
  it('offers confirm to a candidate, retire to an active fact, both to a suspect one', () => {
    expect([wordsFor('candidate'), wordsFor('active'), wordsFor('suspect')]).toEqual([
      ['confirm'],
      ['retire'],
      ['confirm', 'retire'],
    ]);
    expect([wordLabel('confirm', 'candidate'), wordLabel('confirm', 'suspect')]).toEqual([
      'Confirm',
      'Confirm again',
    ]);
    expect(retireQuestion(8)).toBe(
      'Retire fact 8? Cortex will stop showing it here and stop giving it to agents.',
    );
  });

  it("says what came of it, in the daemon's words when it refused", () => {
    expect(outcomeSentence(3, 'confirm', { kind: 'done', status: 'active' })).toBe(
      "Fact 3 confirmed: it is active now, and Cortex may give it to agents when its evidence passes Cortex's checks.",
    );
    expect(outcomeSentence(8, 'retire', { kind: 'done', status: 'retired' })).toBe(
      'Fact 8 retired: Cortex no longer shows it or gives it to agents.',
    );
    expect(
      outcomeSentence(7, 'confirm', {
        kind: 'undone',
        why: 'refused',
        reason: 'there is no fact 7',
      }),
    ).toBe('Cortex refused to confirm fact 7: there is no fact 7.');
    expect(outcomeSentence(7, 'retire', { kind: 'undone', why: 'busy', reason: 'try again' })).toBe(
      'Cortex did not say fact 7 was retired: try again.',
    );
    expect(
      outcomeSentence(7, 'confirm', {
        kind: 'undone',
        why: 'unwritten',
        reason: 'the store failed to write it, which changed nothing',
      }),
    ).toBe('Fact 7 was not confirmed: the store failed to write it, which changed nothing.');
    expect(
      outcomeSentence(7, 'retire', {
        kind: 'unanswered',
        error: { code: 'network', message: 'Failed to fetch' },
      }),
    ).toBe(
      'No answer came to the word on fact 7 (network): Failed to fetch. Cortex may have stopped; if it runs, reload to see whether it was done.',
    );
    expect(
      outcomeSentence(7, 'confirm', {
        kind: 'unread',
        error: { code: 'bad_json', message: 'Unexpected end of JSON input' },
      }),
    ).toBe(
      "Cortex's answer on fact 7 could not be read (bad_json): Unexpected end of JSON input. Reload to see whether it was done.",
    );
  });
});

describe("the file's symbols", () => {
  const order: FileSymbol = {
    name: 'OrderBook',
    kind: 'class',
    start_line: 40,
    end_line: 120,
    is_nested: false,
    selected: 2,
  };

  it('say each symbol and span with its lines and how many work orders selected it', () => {
    expect(symbolSentence(order)).toBe(
      'OrderBook, a class at lines 40 to 120, selected by 2 work orders.',
    );
    expect(
      symbolSentence({
        ...order,
        name: 'cancel',
        kind: 'method',
        start_line: 88,
        end_line: 88,
        is_nested: true,
        selected: 0,
      }),
    ).toBe('cancel, a method inside another symbol at line 88.');
    expect(symbolSentence({ ...order, name: 'Ledger', kind: 'interface', selected: 0 })).toBe(
      'Ledger, an interface at lines 40 to 120.',
    );
    expect(spanSentence({ start_line: 40, end_line: 88, orders: 1 })).toBe(
      'Lines 40 to 88, selected by 1 work order.',
    );
  });

  it('say how many symbols there are and how many are highlighted', () => {
    expect(
      symbolsNote({
        symbols: [order, { ...order, selected: 0 }],
        symbols_total: 640,
        selected: [{ start_line: 40, end_line: 88, orders: 2 }],
      }),
    ).toBe(
      'The first 2 of 640 symbols, in line order, as Cortex last indexed the file. Highlighted: 1 symbol holding lines the work orders that listed this file (the latest 20 at most) selected.',
    );
    expect(symbolsNote({ symbols: [order], symbols_total: 1, selected: [] })).toBe(
      '1 symbol, in line order, as Cortex last indexed the file. None of the work orders that listed this file (the latest 20 at most) selected lines of it.',
    );
  });
});
