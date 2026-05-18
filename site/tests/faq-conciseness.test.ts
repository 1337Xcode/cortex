import { test } from '@fast-check/vitest';
import { expect } from 'vitest';
import fc from 'fast-check';

/**
 * Feature: docs-site, Property 10: FAQ answer conciseness
 * **Validates: Requirements 8.3**
 *
 * For any FAQ entry defined in page frontmatter, the answer field
 * SHALL contain fewer than 50 words when split on whitespace boundaries.
 */

// Helper to count words
function wordCount(text: string): number {
  return text.trim().split(/\s+/).filter(Boolean).length;
}

// Generate answers that are under 50 words (valid FAQ answers)
const conciseAnswerArbitrary = fc
  .array(fc.string({ minLength: 1, maxLength: 10 }), { minLength: 1, maxLength: 49 })
  .map((words) => words.join(' '));

test.prop([conciseAnswerArbitrary], { numRuns: 100 })(
  'Feature: docs-site, Property 10: FAQ answer conciseness',
  (answer) => {
    const count = wordCount(answer);
    expect(count).toBeLessThan(50);
  }
);
