// The map's GPU programs are prepared while the snapshot loads. Every WebGL link the page makes is
// recorded with its context and its two shader sources, and the snapshot is held back until the
// hidden warm-up canvas has started its links, so the check does not depend on timing: the
// warm-up starts every link before it reads any back (so the driver compiles them side by side),
// and the map's own canvas then links only programs the warm-up already linked.
import { expect, test, type Page } from '@playwright/test';
import { largeFixture } from './fixtures';
import { MAP_URL, serveFixture } from './serve';

/** One link or link-status read, as the page recorded it. */
interface GlEvent {
  readonly kind: 'link' | 'read';
  /** Which WebGL context, numbered in the order they first linked or read. */
  readonly context: number;
  /** For a link, the program's vertex and fragment sources joined. */
  readonly sources: string;
}

/** Records links and link-status reads; runs in the page before any of its scripts. */
function recordLinks(): void {
  const proto = WebGL2RenderingContext.prototype;
  const events: GlEvent[] = [];
  const contexts = new WeakMap<object, number>();
  const sources = new WeakMap<object, string>();
  const attached = new WeakMap<object, string[]>();
  let contextCount = 0;
  const contextOf = (gl: WebGL2RenderingContext): number => {
    const known = contexts.get(gl);
    if (known !== undefined) return known;
    contexts.set(gl, contextCount);
    contextCount += 1;
    return contextCount - 1;
  };
  /** Runs `before` on each call of the context method `name`, then the method itself. */
  const watch = (
    name: string,
    before: (gl: WebGL2RenderingContext, args: unknown[]) => void,
  ): void => {
    const original: unknown = Reflect.get(proto, name);
    if (typeof original !== 'function') return;
    Reflect.set(proto, name, function (this: WebGL2RenderingContext, ...args: unknown[]): unknown {
      before(this, args);
      const value: unknown = Reflect.apply(original, this, args);
      return value;
    });
  };
  Reflect.set(window, '__glEvents', events);
  watch('shaderSource', (_gl, [shader, source]) => {
    if (typeof shader === 'object' && shader !== null && typeof source === 'string') {
      sources.set(shader, source);
    }
  });
  watch('attachShader', (_gl, [program, shader]) => {
    if (typeof program !== 'object' || program === null) return;
    if (typeof shader !== 'object' || shader === null) return;
    attached.set(program, [...(attached.get(program) ?? []), sources.get(shader) ?? '']);
  });
  watch('linkProgram', (gl, [program]) => {
    if (typeof program !== 'object' || program === null) return;
    const joined = (attached.get(program) ?? []).join('|');
    events.push({ kind: 'link', context: contextOf(gl), sources: joined });
  });
  watch('getProgramParameter', (gl, [, name]) => {
    if (name === gl.LINK_STATUS) events.push({ kind: 'read', context: contextOf(gl), sources: '' });
  });
}

/** Whether `value` is an event as `recordLinks` writes them. */
function isGlEvent(value: unknown): value is GlEvent {
  if (typeof value !== 'object' || value === null) return false;
  const { kind, context, sources }: { kind?: unknown; context?: unknown; sources?: unknown } =
    value;
  return (
    (kind === 'link' || kind === 'read') &&
    typeof context === 'number' &&
    typeof sources === 'string'
  );
}

/** The recorded events. */
async function glEvents(page: Page): Promise<GlEvent[]> {
  const events: unknown = await page.evaluate((): unknown => Reflect.get(window, '__glEvents'));
  return Array.isArray(events) ? events.filter(isGlEvent) : [];
}

test('the warm-up links every program side by side and the map links nothing new', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.addInitScript(recordLinks);
  await serveFixture(page, largeFixture(2_000));
  let release = (): void => undefined;
  const released = new Promise<void>((resolve) => {
    release = resolve;
  });
  // Routes added later are asked first: hold the snapshot, then let the fixture answer it.
  await page.route('**/api/graph', async (route) => {
    await released;
    await route.fallback();
  });
  await page.goto(MAP_URL);
  // The warm-up starts all its links in one task, so once one is recorded all of them are.
  await page.waitForFunction(
    () => {
      const events: unknown = Reflect.get(window, '__glEvents');
      return Array.isArray(events) && events.length > 0;
    },
    undefined,
    { timeout: 30_000 },
  );
  release();
  await expect(page.getByTestId('summary-indexed')).toHaveText(/^2,000 files in/);
  await expect(page.getByTestId('map-status')).toBeAttached();

  const events = await glEvents(page);
  const isWarm = (event: GlEvent): boolean => event.context === 0;
  const warmLinks = events.filter((event) => event.kind === 'link' && isWarm(event));
  const mapLinks = events.filter((event) => event.kind === 'link' && !isWarm(event));
  // The map's layers use five programs: circles, squares, dashed lines, glyphs, label backings.
  expect(new Set(warmLinks.map((event) => event.sources)).size).toBe(5);
  const lastWarmLink = events.reduce(
    (last, event, index) => (event.kind === 'link' && isWarm(event) ? index : last),
    -1,
  );
  const firstWarmRead = events.findIndex((event) => event.kind === 'read' && isWarm(event));
  expect(firstWarmRead === -1 || firstWarmRead > lastWarmLink).toBe(true);
  expect(mapLinks.length).toBeGreaterThan(0);
  const warmed = new Set(warmLinks.map((event) => event.sources));
  expect(mapLinks.filter((event) => !warmed.has(event.sources))).toEqual([]);
  expect(errors).toEqual([]);
});
