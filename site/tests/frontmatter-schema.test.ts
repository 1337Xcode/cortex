import { test } from '@fast-check/vitest';
import { expect } from 'vitest';
import fc from 'fast-check';
import { z } from 'zod';

/**
 * Feature: docs-site, Property 12: Frontmatter schema validation
 * **Validates: Requirements 10.4**
 */

// Recreate the frontmatter schema (same as content/config.ts)
const frontmatterSchema = z.object({
  title: z.string(),
  description: z.string(),
  order: z.number(),
  category: z.string().optional().default('general'),
  lastModified: z.string().optional(),
  faq: z.array(z.object({
    question: z.string(),
    answer: z.string(),
  })).optional(),
});

// Generate valid frontmatter objects
const validFrontmatter = fc.record({
  title: fc.string({ minLength: 1 }),
  description: fc.string({ minLength: 1 }),
  order: fc.integer({ min: 1, max: 100 }),
});

test.prop([validFrontmatter], { numRuns: 100 })(
  'Feature: docs-site, Property 12: Frontmatter schema validation - valid objects accepted',
  (data) => {
    const result = frontmatterSchema.safeParse(data);
    expect(result.success).toBe(true);
  }
);

// Generate invalid frontmatter objects (missing required fields or wrong types)
const invalidFrontmatter = fc.oneof(
  // Missing title
  fc.record({ description: fc.string(), order: fc.integer() }).map(({ description, order }) => ({ description, order })),
  // Missing description
  fc.record({ title: fc.string(), order: fc.integer() }).map(({ title, order }) => ({ title, order })),
  // Missing order
  fc.record({ title: fc.string(), description: fc.string() }).map(({ title, description }) => ({ title, description })),
  // Wrong type for order (string instead of number)
  fc.record({ title: fc.string(), description: fc.string(), order: fc.string() }),
);

test.prop([invalidFrontmatter], { numRuns: 100 })(
  'Feature: docs-site, Property 12: Frontmatter schema validation - invalid objects rejected',
  (data) => {
    const result = frontmatterSchema.safeParse(data);
    expect(result.success).toBe(false);
  }
);
