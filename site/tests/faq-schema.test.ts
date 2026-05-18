import { test } from '@fast-check/vitest';
import { expect } from 'vitest';
import fc from 'fast-check';
import { buildFaqSchema } from '../src/lib/structured-data';

/**
 * **Validates: Requirements 8.1**
 */

const faqArbitrary = fc.array(
  fc.record({
    question: fc.string({ minLength: 1 }),
    answer: fc.string({ minLength: 1 }),
  }),
  { minLength: 1, maxLength: 20 }
);

test.prop([faqArbitrary], { numRuns: 100 })(
  'Feature: docs-site, Property 8: FAQ schema generation',
  (faqs) => {
    const schema = buildFaqSchema(faqs);

    // Verify it's a valid FAQPage schema
    expect(schema['@context']).toBe('https://schema.org');
    expect(schema['@type']).toBe('FAQPage');
    expect(schema.mainEntity).toHaveLength(faqs.length);

    // Verify every input question/answer appears in the output
    faqs.forEach((faq, i) => {
      const entity = schema.mainEntity[i];
      expect(entity['@type']).toBe('Question');
      expect(entity.name).toBe(faq.question);
      expect(entity.acceptedAnswer['@type']).toBe('Answer');
      expect(entity.acceptedAnswer.text).toBe(faq.answer);
    });
  }
);
