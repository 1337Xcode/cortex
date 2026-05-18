import { describe, it, expect } from 'vitest';
import { existsSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

/**
 * Integration test: Build output verification
 * Validates: Requirements 1.3, 7.3, 11.2
 */
describe('Build output verification', () => {
  const distDir = resolve(__dirname, '..', 'dist');

  it('dist directory exists', () => {
    expect(existsSync(distDir)).toBe(true);
  });

  it('landing page HTML exists', () => {
    expect(existsSync(resolve(distDir, 'index.html'))).toBe(true);
  });

  it('all 6 docs pages exist', () => {
    const docSlugs = ['getting-started', 'ide-setup', 'cli-reference', 'tools', 'configuration', 'architecture'];
    docSlugs.forEach((slug) => {
      const path = resolve(distDir, 'docs', slug, 'index.html');
      expect(existsSync(path), `Missing: docs/${slug}/index.html`).toBe(true);
    });
  });

  it('visualization page exists', () => {
    expect(existsSync(resolve(distDir, 'visualization', 'index.html'))).toBe(true);
  });

  it('issues page exists', () => {
    expect(existsSync(resolve(distDir, 'issues', 'index.html'))).toBe(true);
  });

  it('sitemap files exist', () => {
    expect(existsSync(resolve(distDir, 'sitemap-index.xml'))).toBe(true);
  });

  it('minified CSS assets exist', () => {
    const astroDir = resolve(distDir, '_astro');
    if (existsSync(astroDir)) {
      const files = readdirSync(astroDir);
      const cssFiles = files.filter((f) => f.endsWith('.css'));
      expect(cssFiles.length).toBeGreaterThan(0);
    }
  });

  it('minified JS assets exist', () => {
    const astroDir = resolve(distDir, '_astro');
    if (existsSync(astroDir)) {
      const files = readdirSync(astroDir);
      const jsFiles = files.filter((f) => f.endsWith('.js'));
      expect(jsFiles.length).toBeGreaterThan(0);
    }
  });

  it('font files are included', () => {
    const fontsDir = resolve(distDir, 'fonts');
    expect(existsSync(fontsDir)).toBe(true);
    const fonts = readdirSync(fontsDir);
    expect(fonts.length).toBeGreaterThanOrEqual(6);
  });
});
