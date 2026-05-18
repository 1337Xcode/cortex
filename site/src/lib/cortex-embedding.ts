/** Deterministic L2-normalized 3D projection of the Cortex name (embedding-style display). */
export function cortexEmbedding3(text = 'Cortex'): [number, number, number] {
  const bytes = new TextEncoder().encode(text);
  let a = 2166136261;
  let b = 16777619 ^ bytes.length;
  let c = 2166136261 ^ bytes[0];

  for (let i = 0; i < bytes.length; i++) {
    const byte = bytes[i];
    a ^= byte;
    a = Math.imul(a, 16777619);
    b ^= byte ^ i;
    b = Math.imul(b, 2166136261);
    c ^= byte ^ (bytes.length - i);
    c = Math.imul(c, 2246822519);
  }

  const toUnit = (n: number) => 2 * ((n >>> 0) / 4294967295) - 1;
  const v: [number, number, number] = [toUnit(a), toUnit(b), toUnit(c)];
  const mag = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / mag, v[1] / mag, v[2] / mag];
}

export function formatEmbedding3(v: [number, number, number]): string {
  return `[${v.map((n) => `${n >= 0 ? '+' : ''}${n.toFixed(3)}`).join(', ')}]`;
}

export function randomEmbedding3(): [number, number, number] {
  const v: [number, number, number] = [
    Math.random() * 2 - 1,
    Math.random() * 2 - 1,
    Math.random() * 2 - 1,
  ];
  const mag = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / mag, v[1] / mag, v[2] / mag];
}
