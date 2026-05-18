import { test } from '@fast-check/vitest';
import { expect, describe, it } from 'vitest';
import fc from 'fast-check';
import { buildWebSiteSchema, buildSoftwareApplicationSchema } from '../src/lib/structured-data';

/**
 * Feature: docs-site, Property 7: Structured data completeness
 *
 * Validates: Requirements 7.5, 9.3
 *
 * For any site configuration, the buildWebSiteSchema and buildSoftwareApplicationSchema
 * functions SHALL produce JSON-LD objects containing all required Schema.org fields
 * (name, url, description, author) populated from the site config, with no null or
 * empty required values.
 */
describe('Feature: docs-site, Property 7: Structured data completeness', () => {
  it('buildWebSiteSchema produces valid JSON-LD with all required fields', () => {
    const schema = buildWebSiteSchema();

    expect(schema['@context']).toBe('https://schema.org');
    expect(schema['@type']).toBe('WebSite');
    expect(schema.name).toBeTruthy();
    expect(schema.url).toBeTruthy();
    expect(schema.description).toBeTruthy();
    expect(schema.author).toBeTruthy();
    expect(schema.author['@type']).toBe('Organization');
    expect(schema.author.name).toBeTruthy();
  });

  it('buildSoftwareApplicationSchema produces valid JSON-LD with all required fields', () => {
    const schema = buildSoftwareApplicationSchema();

    expect(schema['@context']).toBe('https://schema.org');
    expect(schema['@type']).toBe('SoftwareApplication');
    expect(schema.name).toBeTruthy();
    expect(schema.description).toBeTruthy();
    expect(schema.applicationCategory).toBeTruthy();
    expect(schema.operatingSystem).toBeTruthy();
    expect(schema.programmingLanguage).toBeTruthy();
    expect(schema.url).toBeTruthy();
    expect(schema.offers).toBeTruthy();
    expect(schema.offers['@type']).toBe('Offer');
    expect(schema.license).toBeTruthy();
  });

  test.prop([fc.integer({ min: 1, max: 100 })], { numRuns: 100 })(
    'schemas are consistent across multiple invocations',
    () => {
      const webSite1 = buildWebSiteSchema();
      const webSite2 = buildWebSiteSchema();
      const app1 = buildSoftwareApplicationSchema();
      const app2 = buildSoftwareApplicationSchema();

      expect(webSite1).toEqual(webSite2);
      expect(app1).toEqual(app2);

      // Verify no null or empty required values
      expect(webSite1.name).not.toBe('');
      expect(webSite1.url).not.toBe('');
      expect(webSite1.description).not.toBe('');
      expect(app1.name).not.toBe('');
      expect(app1.url).not.toBe('');
      expect(app1.description).not.toBe('');
    }
  );
});
