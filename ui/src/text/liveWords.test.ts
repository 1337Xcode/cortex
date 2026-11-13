// Tests for the live sessions' words, every expected sentence written out.
import { describe, expect, it } from 'vitest';
import type { LiveSession } from '../api/live';
import { joinNames, liveSentence, sessionLead, sessionName, shortSessionId } from './liveWords';

const CLAUDE: LiveSession = { id: '3f2a9c1e', agent: 'claude', title: 'Claude Code', files: [] };

describe('session names', () => {
  it('shortens an id to four characters and an ellipsis, and leaves a short one whole', () => {
    expect([shortSessionId('3f2a9c1e'), shortSessionId('3f2a'), shortSessionId('ab')]).toEqual([
      '3f2a…',
      '3f2a',
      'ab',
    ]);
    expect(sessionName(CLAUDE)).toBe('Claude Code 3f2a…');
    expect(sessionLead(CLAUDE, 'edit')).toBe('Claude Code 3f2a… editing');
  });

  it('shows a hidden formatting character in the title as the replacement character', () => {
    expect(sessionName({ ...CLAUDE, title: 'Claude\u202eCode' })).toBe('Claude�Code 3f2a…');
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
    expect(liveSentence(CLAUDE, files)).toBe(
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
    expect(liveSentence(CLAUDE, files)).toBe(
      'Claude Code session 3f2a… has claimed a.rs; before that it read b.rs and d.rs, and edited c.rs, plus 2 more files.',
    );
  });

  it('names each file as the map names it, and says so when the map holds none of its files', () => {
    const files = [
      { name: 'orders/mod.rs', mode: 'read' as const },
      { name: 'billing/mod.rs', mode: 'read' as const },
    ];
    expect(liveSentence(CLAUDE, files)).toBe(
      'Claude Code session 3f2a… is reading orders/mod.rs; before that it read billing/mod.rs.',
    );
    expect(liveSentence(CLAUDE, [])).toBe(
      'Claude Code session 3f2a… is working on files the map does not show yet.',
    );
  });
});
