// Byte order for strings. The daemon sorts paths by their UTF-8 bytes, which is the order of
// Unicode code points. JavaScript's `<` compares UTF-16 units and disagrees for characters
// outside the Basic Multilingual Plane, so the map compares with this function instead.

const SURROGATE_FIRST = 0xd800;
const SURROGATE_LAST = 0xdfff;

function isSurrogate(unit: number): boolean {
  return unit >= SURROGATE_FIRST && unit <= SURROGATE_LAST;
}

/**
 * Compares two strings in UTF-8 byte order. Returns a negative number when `a` sorts first,
 * a positive number when `b` sorts first and 0 when they are equal.
 */
export function compareBytes(a: string, b: string): number {
  const shared = Math.min(a.length, b.length);
  for (let i = 0; i < shared; i += 1) {
    const unitA = a.charCodeAt(i);
    const unitB = b.charCodeAt(i);
    if (unitA === unitB) continue;
    // A surrogate starts a code point above U+FFFF, which sorts after every other unit,
    // including U+E000 to U+FFFF that compare higher as raw UTF-16 units.
    const surrogateA = isSurrogate(unitA);
    if (surrogateA !== isSurrogate(unitB)) return surrogateA ? 1 : -1;
    return unitA - unitB;
  }
  return a.length - b.length;
}

/** Whether the strings are in strictly increasing byte order (sorted, no duplicates). */
export function isStrictlySortedBytes(values: readonly string[]): boolean {
  for (let i = 1; i < values.length; i += 1) {
    const previous = values[i - 1];
    const current = values[i];
    if (previous === undefined || current === undefined) return false;
    if (compareBytes(previous, current) >= 0) return false;
  }
  return true;
}
