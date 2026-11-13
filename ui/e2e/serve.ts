// Serves the built page and a fixture's API to a Playwright page without a server: every request
// to the map's origin is answered from `dist/` or the fixture, under the same `/<token>/` prefix
// the daemon uses, and the live view's WebSocket is answered by a stand-in that sends the
// fixture's live state on connect and whatever frames a test pushes after. Any request to another
// origin is aborted and recorded, so a test can assert that nothing left the page's origin. Every
// response carries the daemon's security headers, so the page is checked under the same content
// security policy it runs under for real. A POST is answered as the daemon's guard and its facts
// endpoint answer it: refused without the map's own `Origin` header as the browser sent it, else
// the word on the fact carried out on the fixture's panels, which later reads of `api/file` show.
// This proves what the browser sends; that the real daemon takes it must be checked against one.
import { join } from 'node:path';
import type { Page, Route, WebSocketRoute } from '@playwright/test';
import { fileDetail, type Fixture, type FixtureSymbol, type Json } from './fixtures';

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
  'referrer-policy': 'same-origin',
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
    woff2: 'font/woff2',
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

/** The body for one API path, or null for a 404; `details` are the files' panels as they stand. */
function apiBody(
  fixture: Fixture,
  bodies: { graph: string; summary: string },
  details: ReadonlyMap<number, Json>,
  url: URL,
): string | null {
  const path = url.pathname.replace(/^\/0123456789abcdef\//, '');
  if (path === 'api/graph') return bodies.graph;
  if (path === 'api/summary') return bodies.summary;
  if (path === 'api/health') return JSON.stringify(fixture.health);
  if (path === 'api/receipts') return JSON.stringify(fixture.receipts);
  if (path === 'api/activity') return JSON.stringify(fixture.activity);
  if (path === 'api/warnings') return JSON.stringify(fixture.warnings);
  if (path === 'api/search') {
    return JSON.stringify(searchFixture(fixture, url.searchParams.get('q') ?? ''));
  }
  const match = /^api\/file\/(\d+)$/.exec(path);
  if (match === null) return null;
  const id = Number(match[1]);
  const file = fixture.graph.files[id];
  if (file === undefined) return null;
  return JSON.stringify(details.get(id) ?? fileDetail(id, file.path));
}

/** A POST the page made, as the stand-in daemon saw it. */
export interface SeenPost {
  readonly path: string;
  /** The request's `Origin` header as the browser sent it, or null without one. */
  readonly origin: string | null;
  /** Whether it carried a body. */
  readonly hasBody: boolean;
}

/** The stand-in daemon's facts endpoint of one page, which a test reads and steers. */
export interface FactsChannel {
  /** Every POST the page made, in order. */
  readonly posts: () => readonly SeenPost[];
  /** Answers the next word with `status` and `body` instead of carrying it out. */
  readonly answerNext: (status: number, body: Json) => void;
}

/** Each served page's facts channel. */
const FACT_CHANNELS = new WeakMap<Page, FactsChannel>();

/** The facts channel of a page `serveFixture` served. */
export function factsChannel(page: Page): FactsChannel {
  const channel = FACT_CHANNELS.get(page);
  if (channel === undefined) throw new Error('the page was not served by serveFixture');
  return channel;
}

/** A fact as the fixtures write them. */
interface FixtureFact {
  readonly id: number;
  readonly status: string;
  readonly [key: string]: Json;
}

/** Whether `value` is an object holding a numbered fact. */
function isFact(value: Json): value is FixtureFact {
  return (
    typeof value === 'object' &&
    value !== null &&
    !Array.isArray(value) &&
    typeof value['id'] === 'number' &&
    typeof value['status'] === 'string'
  );
}

/**
 * Carries out `word` on fact `id` in `details` as the daemon's writer does: confirm makes it
 * active, retire takes it off the panel; the answer is the daemon's body and status.
 */
function carryOut(
  details: Map<number, Json>,
  id: number,
  word: string,
): { status: number; body: Json } {
  for (const [file, detail] of details) {
    if (typeof detail !== 'object' || detail === null || Array.isArray(detail)) continue;
    const facts = detail['facts'];
    if (!Array.isArray(facts) || !facts.some((fact) => isFact(fact) && fact.id === id)) continue;
    const kept = facts.flatMap((fact): Json[] => {
      if (!isFact(fact) || fact.id !== id) return [fact];
      return word === 'retire' ? [] : [{ ...fact, status: 'active' }];
    });
    details.set(file, { ...detail, facts: kept });
    const status = word === 'retire' ? 'retired' : 'active';
    return { status: 200, body: { version: 1, id, status } };
  }
  return { status: 409, body: { version: 1, error: `there is no fact ${id}` } };
}

/**
 * Answers a POST as the daemon's guard and its facts endpoint do: refused (403) without the map's
 * own `Origin`, 404 for any path but a word on a fact, then the word carried out on `details`.
 */
async function answerPost(
  route: Route,
  details: Map<number, Json>,
  seen: { posts: SeenPost[]; next: { status: number; body: Json } | null },
): Promise<void> {
  const request = route.request();
  const url = new URL(request.url());
  const path = url.pathname.replace(/^\/0123456789abcdef\//, '');
  const headers = await request.allHeaders();
  const origin = headers['origin'] ?? null;
  seen.posts.push({ path, origin, hasBody: (request.postData() ?? '') !== '' });
  const json = (status: number, body: Json): Promise<void> =>
    route.fulfill({
      status,
      headers: { ...SECURITY_HEADERS, 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
  if (origin !== MAP_ORIGIN) {
    await route.fulfill({ status: 403, headers: SECURITY_HEADERS, body: '' });
    return;
  }
  const match = /^api\/facts\/(\d+)\/(confirm|retire)$/.exec(path);
  if (match === null) {
    await route.fulfill({ status: 404, headers: SECURITY_HEADERS, body: '' });
    return;
  }
  const queued = seen.next;
  seen.next = null;
  const answer = queued ?? carryOut(details, Number(match[1]), match[2] ?? '');
  await json(answer.status, answer.body);
}

/** The live view's stand-in server of one page, which a test drives. */
export interface LiveChannel {
  /** Sends `frame` as text to every socket the page holds open. */
  readonly push: (frame: Json | string) => void;
  /** Closes every open socket, as a daemon that stopped would. */
  readonly closeAll: () => void;
  /**
   * While true, a socket the page opens is closed at once, so the live view stays paused; while
   * false, it is answered with the last state pushed (the fixture's at first).
   */
  readonly setRefusing: (isRefusing: boolean) => void;
  /** How many sockets the page has opened. */
  readonly openedCount: () => number;
}

/** Each served page's live channel. */
const CHANNELS = new WeakMap<Page, LiveChannel>();

/** The live channel of a page `serveFixture` served. */
export function liveChannel(page: Page): LiveChannel {
  const channel = CHANNELS.get(page);
  if (channel === undefined) throw new Error('the page was not served by serveFixture');
  return channel;
}

/** Answers the page's `api/live` socket with `first`, and returns the channel that drives it. */
async function routeLive(page: Page, first: Json): Promise<LiveChannel> {
  const open = new Set<WebSocketRoute>();
  let last = JSON.stringify(first);
  let isRefusing = false;
  let opened = 0;
  await page.routeWebSocket(/\/api\/live$/, (socket) => {
    opened += 1;
    if (isRefusing) {
      void socket.close();
      return;
    }
    open.add(socket);
    socket.onClose(() => open.delete(socket));
    socket.send(last);
  });
  return {
    push: (frame) => {
      last = typeof frame === 'string' ? frame : JSON.stringify(frame);
      for (const socket of open) socket.send(last);
    },
    closeAll: () => {
      for (const socket of open) void socket.close();
      open.clear();
    },
    setRefusing: (value) => {
      isRefusing = value;
    },
    openedCount: () => opened,
  };
}

/** Routes every request of `page`; returns the list that collects requests to other origins. */
export async function serveFixture(page: Page, fixture: Fixture): Promise<string[]> {
  CHANNELS.set(page, await routeLive(page, fixture.live));
  const foreign: string[] = [];
  const bodies = { graph: JSON.stringify(fixture.graph), summary: JSON.stringify(fixture.summary) };
  const details = new Map(fixture.details);
  const seen: { posts: SeenPost[]; next: { status: number; body: Json } | null } = {
    posts: [],
    next: null,
  };
  FACT_CHANNELS.set(page, {
    posts: () => seen.posts,
    answerNext: (status, body) => {
      seen.next = { status, body };
    },
  });
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== MAP_ORIGIN) {
      foreign.push(url.href);
      await route.abort();
      return;
    }
    if (route.request().method() === 'POST') {
      await answerPost(route, details, seen);
      return;
    }
    const path = url.pathname.replace(/^\/0123456789abcdef\//, '');
    if (path.startsWith('api/')) {
      const body = apiBody(fixture, bodies, details, url);
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
