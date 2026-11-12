// Path generators for the layout tests: deterministic trees of any size with uneven directories.
import { compareBytes } from '../text/byteOrder';

/** A small seeded generator (mulberry32), so every run builds the same tree. */
function seededRandom(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Generates `count` distinct paths up to four directories deep, drawing each directory name
 * from `poolSize` names, so some directories hold many files and others one. Sorted in byte order.
 */
export function generatePaths(count: number, poolSize: number, seed = 1): string[] {
  const random = seededRandom(seed);
  const paths: string[] = [];
  for (let i = 0; i < count; i += 1) {
    const depth = Math.floor(random() * 4);
    const segments: string[] = [];
    for (let level = 0; level < depth; level += 1) {
      segments.push(`d${Math.floor(random() * poolSize)}`);
    }
    segments.push(`f${i}.ts`);
    paths.push(segments.join('/'));
  }
  return paths.sort(compareBytes);
}

/** A deterministic shuffle (Fisher-Yates driven by the seeded generator). */
export function shuffled<T>(values: readonly T[], seed = 7): T[] {
  const random = seededRandom(seed);
  const result = [...values];
  for (let i = result.length - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    const a = result[i];
    const b = result[j];
    if (a === undefined || b === undefined) continue;
    result[i] = b;
    result[j] = a;
  }
  return result;
}
