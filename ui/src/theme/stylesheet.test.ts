// The stylesheets hold no colour or length of their own: every one is a custom property written
// from the tokens (see applyTheme.ts). Each CSS file under src is scanned, comments removed.
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SOURCE = join(import.meta.dirname, '..');

/** Every CSS file under `folder`, by path. */
function cssFiles(folder: string): string[] {
  return readdirSync(folder, { withFileTypes: true }).flatMap((entry) => {
    const path = join(folder, entry.name);
    if (entry.isDirectory()) return cssFiles(path);
    return entry.name.endsWith('.css') ? [path] : [];
  });
}

/** The raw values in `css`, comments left out: written colours and lengths. */
function rawValues(css: string): string[] {
  const code = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const colour = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/g;
  const length = /(?<![\w-])-?\d*\.?\d+(?:px|rem|em|pt|vh|vw|ch|ex)\b/g;
  return [...(code.match(colour) ?? []), ...(code.match(length) ?? [])];
}

describe('the stylesheets', () => {
  it('finds a written colour or length when there is one', () => {
    expect(
      rawValues('a { color: #fff; margin: 12px; width: 1.5rem; fill: oklch(0.5 0 0) }'),
    ).toEqual(['#fff', 'oklch(', '12px', '1.5rem']);
    expect(
      rawValues('/* 12px */ a { margin: var(--space-md); width: 100%; rotate: 90deg }'),
    ).toEqual([]);
  });

  it('write no colour or length outside the tokens', () => {
    const files = cssFiles(SOURCE);
    expect(files.length).toBeGreaterThan(0);
    for (const file of files) expect(rawValues(readFileSync(file, 'utf8')), file).toEqual([]);
  });
});
