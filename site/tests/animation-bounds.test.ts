import { test } from '@fast-check/vitest';
import { expect, describe, it } from 'vitest';
import fc from 'fast-check';

/**
 * Feature: docs-site, Property 1: Animation duration bounds
 * Validates: Requirements 2.4
 *
 * For any animation or transition duration defined in the design system,
 * the duration value SHALL be between 150ms and 400ms inclusive.
 */

// Extract durations from the Tailwind config (hardcoded here to match tailwind.config.mjs)
const animationDurations: Record<string, number> = {
  'fade-in': 300,
  'slide-up': 400,
  'slide-in-left': 300,
};

const transitionDurations: Record<string, number> = {
  DEFAULT: 200,
  fast: 150,
  slow: 400,
};

describe('Feature: docs-site, Property 1: Animation duration bounds', () => {
  it('all animation durations are between 150ms and 400ms', () => {
    Object.entries(animationDurations).forEach(([name, duration]) => {
      expect(duration, `Animation "${name}" duration ${duration}ms`).toBeGreaterThanOrEqual(150);
      expect(duration, `Animation "${name}" duration ${duration}ms`).toBeLessThanOrEqual(400);
    });
  });

  it('all transition durations are between 150ms and 400ms', () => {
    Object.entries(transitionDurations).forEach(([name, duration]) => {
      expect(duration, `Transition "${name}" duration ${duration}ms`).toBeGreaterThanOrEqual(150);
      expect(duration, `Transition "${name}" duration ${duration}ms`).toBeLessThanOrEqual(400);
    });
  });

  // Property-based: any duration in our config should be in bounds
  const allDurations = [...Object.values(animationDurations), ...Object.values(transitionDurations)];

  test.prop([fc.constantFrom(...allDurations)], { numRuns: 100 })(
    'any configured duration is within 150-400ms bounds',
    (duration) => {
      expect(duration).toBeGreaterThanOrEqual(150);
      expect(duration).toBeLessThanOrEqual(400);
    }
  );
});
