// Tests for the live sessions' words, every expected sentence written out.
import { describe, expect, it } from 'vitest';
import type { LiveSession } from '../api/live';
import { joinNames, liveSentence, sessionLead, sessionName } from './liveWords';
import { sessionNames } from './sessionNames';

const CLAUDE: LiveSession = { id: '3f2a9c1e', agent: 'claude', title: 'Claude Code', files: [] };

/** The names of a page that knows only of `CLAUDE`. */
const NAMES = sessionNames([CLAUDE.id]);

describe('session names', () => {
  it('names a session by its title and its id cut to four characters and an ellipsis', () => {
    expect(sessionName(CLAUDE, NAMES)).toBe('Claude Code 3f2a…');
    expect(sessionLead(CLAUDE, 'edit', NAMES)).toBe('Claude Code 3f2a… editing');
  });

  it('names two sessions whose ids start alike by what tells them apart', () => {
    const first: LiveSession = { ...CLAUDE, id: '01a1d373-f6af-7e0b', title: 'Codex' };
    const second: LiveSession = { ...first, id: '01a1d375-b58f-7a41' };
    const names = sessionNames([first.id, second.id]);
    expect(sessionName(first, names)).toBe('Codex 01a1d373…');
    expect(sessionName(second, names)).toBe('Codex 01a1d375…');
    expect(sessionLead(second, 'read', names)).toBe('Codex 01a1d375… reading');
  });

  it('shows a hidden formatting character in the title as the replacement character', () => {
    expect(sessionName({ ...CLAUDE, title: 'Claude\u202eCode' }, NAMES)).toBe('Claude�Code 3f2a…');
  });

  it('joins names as a reader says them', () => {
    expect([
      joinNames([]),
      joinNames(['a']),
      joinNames(['a', 'b']),
      joinNames(['a', 'b', 'c']),
    ]).toEqual(['', 'a', 'a and b', 'a, b and c']);
  });
});

describe('liveSentence', () => {
  it('says what the session does now and what it read before', () => {
    const files = [
      { name: 'book.rs', mode: 'edit' as const },
      { name: 'mod.rs', mode: 'read' as const },
      { name: 'payment.rs', mode: 'read' as const },
    ];
    expect(liveSentence(CLAUDE, files, NAMES)).toBe(
      'Claude Code session 3f2a… is editing book.rs; before that it read mod.rs and payment.rs.',
    );
  });

  it('groups earlier files by what was done, names three and counts the rest', () => {
    const files = [
      { name: 'a.rs', mode: 'claim' as const },
      { name: 'b.rs', mode: 'read' as const },
      { name: 'c.rs', mode: 'edit' as const },
      { name: 'd.rs', mode: 'read' as const },
      { name: 'e.rs', mode: 'read' as const },
      { name: 'f.rs', mode: 'read' as const },
    ];
    expect(liveSentence(CLAUDE, files, NAMES)).toBe(
      'Claude Code session 3f2a… has claimed a.rs; before that it read b.rs and d.rs, and edited c.rs, plus 2 more files.',
    );
  });

  it('names each file as the map names it, and says so when the map holds none of its files', () => {
    const files = [
      { name: 'orders/mod.rs', mode: 'read' as const },
      { name: 'billing/mod.rs', mode: 'read' as const },
    ];
    expect(liveSentence(CLAUDE, files, NAMES)).toBe(
      'Claude Code session 3f2a… is reading orders/mod.rs; before that it read billing/mod.rs.',
    );
    expect(liveSentence(CLAUDE, [], NAMES)).toBe(
      'Claude Code session 3f2a… is working on files the map does not show yet.',
    );
  });
});
