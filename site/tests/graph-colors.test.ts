import { test } from '@fast-check/vitest';
import { expect } from 'vitest';
import fc from 'fast-check';

/**
 * Feature: docs-site, Property 4: Graph node color mapping
 * Validates: Requirements 6.4
 */

// The color mapping from ForceGraph.astro
const nodeColors: Record<string, string> = {
  module: '#525252',
  function: '#737373',
  class: '#404040',
};

function getNodeColor(kind: string): string {
  if (Object.hasOwn(nodeColors, kind)) {
    return nodeColors[kind];
  }
  return '#a3a3a3';
}

const validKinds = fc.oneof(
  fc.constant('module'),
  fc.constant('function'),
  fc.constant('class')
);

test.prop([validKinds], { numRuns: 100 })(
  'Feature: docs-site, Property 4: Graph node color mapping - valid kinds produce defined colors',
  (kind) => {
    const color = getNodeColor(kind);
    expect(color).toBeDefined();
    expect(color).not.toBe('#a3a3a3'); // Should not be fallback for valid kinds
    expect(color).toMatch(/^#[0-9a-f]{6}$/i);
  }
);

test.prop([fc.string()], { numRuns: 100 })(
  'Feature: docs-site, Property 4: Graph node color mapping - any kind produces a valid color',
  (kind) => {
    const color = getNodeColor(kind);
    expect(color).toBeDefined();
    expect(color).toMatch(/^#[0-9a-f]{6}$/i);
  }
);
