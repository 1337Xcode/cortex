// Serves the built page and a fixture's API to a Playwright page without a server: every request
// to the map's origin is answered from `dist/` or the fixture, under the same `/<token>/` prefix
// the daemon uses. Any request to another origin is aborted and recorded, so a test can assert
// that nothing left the page's origin. Every response carries the daemon's security headers, so the
// page is checked under the same content security policy it runs under for real.
import { join } from 'node:path';
import type { Page } from '@playwright/test';
import type { Fixture, FixtureSymbol, Json } from './fixtures';

/** The origin and token the page is served under. */
export const MAP_ORIGIN = 'http://127.0.0.1:47613';
export const MAP_URL = `${MAP_ORIGIN}/0123456789abcdef/`;

const DIST = join(import.meta.dirname, '..', 'dist');

const MAP_PORT = new URL(MAP_ORIGIN).port;

/** The headers the daemon sends with every response of the map's server. */
export const SECURITY_HEADERS: Readonly<Record<string, string>> = {
  'content-security-policy':
    "default-src 'self'; " +
    `connect-src 'self' ws://127.0.0.1:${MAP_PORT} ws://localhost:${MAP_PORT}; ` +
    "base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'",
  'referrer-policy': 'no-referrer',
  'x-content-type-options': 'nosniff',
};

/** The content type of a built file, as the daemon serves it (the page requires it: nosniff). */
function contentTypeOf(path: string): string {
  const extension = path.slice(path.lastIndexOf('.') + 1);
  const types: Readonly<Record<string, string>> = {
    html: 'text/html; charset=utf-8',
    js: 'text/javascript; charset=utf-8',
    css: 'text/css; charset=utf-8',
    svg: 'image/svg+xml',
    json: 'application/json',
  };
  return types[extension] ?? 'application/octet-stream';
}

/** Ranks a name against a query as the daemon does: exact 0, prefix 1, contains 2, else -1. */
function rankOf(name: string, query: string): number {
  const lower = name.toLowerCase();
  if (lower === query) return 0;
  if (lower.startsWith(query)) return 1;
  return lower.includes(query) ? 2 : -1;
}

/** The fixture's search: files whose path holds the text and symbols whose name does, ranked. */
export function searchFixture(fixture: Fixture, text: string): Json {
  const query = text.toLowerCase();
  type Hit = {
    kind: string;
    file: number;
    name: string;
    line: number | null;
    rank: number;
    path: string;
  };
  const hits: Hit[] = [];
  for (const entry of fixture.graph.files) {
    if (!entry.path.toLowerCase().includes(query)) continue;
    const name = entry.path.slice(entry.path.lastIndexOf('/') + 1);
    hits.push({
      kind: 'file',
      file: entry.id,
      name,
      line: null,
      rank: Math.max(rankOf(name, query), 2),
      path: entry.path,
    });
  }
  for (const symbol of fixture.symbols) hits.push(symbolHit(fixture, symbol, query));
  const found = hits.filter((hit) => hit.rank >= 0);
  found.sort(
    (a, b) =>
      a.rank - b.rank ||
      (a.path < b.path ? -1 : a.path > b.path ? 1 : 0) ||
      (a.line ?? 0) - (b.line ?? 0),
  );
  const results = found
    .slice(0, 20)
    .map(({ kind, file, name, line }) => ({ kind, file, name, line }));
  return { version: 1, query: text, results };
}

function symbolHit(
  fixture: Fixture,
  symbol: FixtureSymbol,
  query: string,
): { kind: string; file: number; name: string; line: number; rank: number; path: string } {
  const path = fixture.graph.files[symbol.file]?.path ?? '';
  return {
    kind: 'symbol',
    file: symbol.file,
    name: symbol.name,
    line: symbol.line,
    rank: rankOf(symbol.name, query),
    path,
  };
}

function emptyDetail(fixture: Fixture, id: number): Json {
  const path = fixture.graph.files[id]?.path ?? '';
  return { version: 1, id, path, facts: [], history: [], told: [] };
}

/** The body for one API path, or null for a 404. */
function apiBody(
  fixture: Fixture,
  bodies: { graph: string; summary: string },
  path: string,
  search: URLSearchParams,
): string | null {
  if (path === 'api/graph') return bodies.graph;
  if (path === 'api/summary') return bodies.summary;
  if (path === 'api/search') return JSON.stringify(searchFixture(fixture, search.get('q') ?? ''));
  const match = /^api\/file\/(\d+)$/.exec(path);
  if (match === null) return null;
  const id = Number(match[1]);
  if (id >= fixture.graph.files.length) return null;
  return JSON.stringify(fixture.details.get(id) ?? emptyDetail(fixture, id));
}

/** Routes every request of `page`; returns the list that collects requests to other origins. */
export async function serveFixture(page: Page, fixture: Fixture): Promise<string[]> {
  const foreign: string[] = [];
  const bodies = { graph: JSON.stringify(fixture.graph), summary: JSON.stringify(fixture.summary) };
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== MAP_ORIGIN) {
      foreign.push(url.href);
      await route.abort();
      return;
    }
    const path = url.pathname.replace(/^\/0123456789abcdef\//, '');
    if (path.startsWith('api/')) {
      const body = apiBody(fixture, bodies, path, url.searchParams);
      await route.fulfill(
        body === null
          ? { status: 404, headers: SECURITY_HEADERS, body: '' }
          : {
              status: 200,
              headers: { ...SECURITY_HEADERS, 'content-type': 'application/json' },
              body,
            },
      );
      return;
    }
    const file = path === '' ? 'index.html' : path;
    await route.fulfill({
      path: join(DIST, file),
      headers: { ...SECURITY_HEADERS, 'content-type': contentTypeOf(file) },
    });
  });
  return foreign;
}
