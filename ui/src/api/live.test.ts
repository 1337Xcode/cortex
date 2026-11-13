// Tests for the live frame guard: accepted frames of each shape, then one refused frame per rule
// of the contract with the literal error code it must give.
import { describe, expect, it } from 'vitest';
import type { ApiErrorCode } from './guard';
import { parseLive } from './live';

function file(id: number, mode: string, lastMs: number): Record<string, unknown> {
  return { file: id, path: `src/f${id}.rs`, mode, last_ms: lastMs };
}

function session(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: '3f2a9c1e',
    agent: 'claude',
    title: 'Claude Code',
    files: [file(12, 'edit', 1794470400000), file(3, 'read', 1794470300000)],
    ...overrides,
  };
}

function frame(sessions: unknown[] = [session()]): Record<string, unknown> {
  return { version: 1, sessions };
}

describe('parseLive accepts', () => {
  it('a frame with one session, keeping every field', () => {
    expect(parseLive(frame())).toEqual({ ok: true, value: frame() });
  });

  it('no session at all', () => {
    expect(parseLive(frame([]))).toEqual({ ok: true, value: frame([]) });
  });

  it('sessions in byte order, every mode, equal times, ten files and an agent Cortex names other', () => {
    const ten = Array.from({ length: 10 }, (_, i) =>
      file(i, ['read', 'edit', 'claim'][i % 3] ?? '', 5),
    );
    const sessions = [
      session({ id: 'A1', files: [] }),
      session({ id: 'Z run été', files: [] }),
      session({ id: 'a1', agent: 'other', title: 'Aider', files: ten }),
      session({ id: 'a2', agent: 'gemini_cli', title: 'Gemini CLI' }),
    ];
    expect(parseLive(frame(sessions)).ok).toBe(true);
  });
});

const REFUSED: [string, unknown, ApiErrorCode][] = [
  ['text instead of an object', 'live', 'bad_shape'],
  ['an unknown version', { version: 2, sessions: [] }, 'bad_version'],
  ['missing sessions', { version: 1 }, 'bad_shape'],
  ['sessions out of order', frame([session({ id: 'b' }), session({ id: 'a' })]), 'bad_order'],
  ['the same session twice', frame([session(), session()]), 'bad_order'],
  ['a session id with a line break', frame([session({ id: '3f2a\n9c1e' })]), 'bad_shape'],
  ['an empty session id', frame([session({ id: '' })]), 'bad_shape'],
  ['a session id of 257 characters', frame([session({ id: 'x'.repeat(257) })]), 'bad_range'],
  ['an agent that is not a snake name', frame([session({ agent: 'Claude Code' })]), 'bad_shape'],
  ['an agent of 33 characters', frame([session({ agent: 'a'.repeat(33) })]), 'bad_range'],
  ['an empty title', frame([session({ title: '' })]), 'bad_shape'],
  ['a title with a control character', frame([session({ title: 'Claude\nCode' })]), 'bad_shape'],
  ['a title of 65 characters', frame([session({ title: 'C'.repeat(65) })]), 'bad_range'],
  ['files that are not a list', frame([session({ files: {} })]), 'bad_shape'],
  [
    'eleven files',
    frame([session({ files: Array.from({ length: 11 }, (_, i) => file(i, 'read', 1)) })]),
    'bad_range',
  ],
  [
    'files oldest first',
    frame([session({ files: [file(1, 'read', 1), file(2, 'read', 2)] })]),
    'bad_order',
  ],
  ['an unknown mode', frame([session({ files: [file(1, 'write', 1)] })]), 'bad_kind'],
  ['a negative file id', frame([session({ files: [file(-1, 'read', 1)] })]), 'bad_shape'],
  ['a fractional file id', frame([session({ files: [file(1.5, 'read', 1)] })]), 'bad_shape'],
  [
    'a time past the latest date',
    frame([session({ files: [file(1, 'read', 9e15)] })]),
    'bad_range',
  ],
  [
    'a missing time',
    frame([session({ files: [{ file: 1, path: 'a.rs', mode: 'read' }] })]),
    'bad_shape',
  ],
  [
    'a missing path',
    frame([session({ files: [{ file: 1, mode: 'read', last_ms: 1 }] })]),
    'bad_shape',
  ],
  [
    'a path out of the repository',
    frame([session({ files: [{ file: 1, path: '/etc/passwd', mode: 'read', last_ms: 1 }] })]),
    'bad_shape',
  ],
];

describe('parseLive refuses', () => {
  it.each(REFUSED)('%s', (_name, input, code) => {
    const result = parseLive(input);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});
