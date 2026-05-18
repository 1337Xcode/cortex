import { describe, it, expect } from 'vitest';
import { withBase, canonicalPath } from '../src/lib/paths';
import { siteConfig } from '../site.config';

describe('withBase', () => {
  it('prefixes internal paths with Astro base', () => {
    const result = withBase('/docs/getting-started/');
    const expectedBase = import.meta.env.BASE_URL.replace(/\/$/, '');
    if (expectedBase && expectedBase !== '') {
      expect(result).toBe(`${expectedBase}/docs/getting-started/`);
    } else {
      expect(result).toBe('/docs/getting-started/');
    }
  });
});

describe('canonicalPath', () => {
  it('builds absolute-style paths under site base', () => {
    const path = canonicalPath('docs/tools');
    expect(path).toContain('docs/tools');
  });
});

describe('siteConfig', () => {
  it('uses GitHub Pages URL', () => {
    expect(siteConfig.url).toBe('https://1337xcode.github.io/cortex');
  });
});
