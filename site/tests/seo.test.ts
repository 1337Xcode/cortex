import { test } from '@fast-check/vitest';
import { expect } from 'vitest';
import fc from 'fast-check';
import { buildSeoMeta } from '../src/lib/seo';
import { siteConfig } from '../site.config';

/**
 * Feature: docs-site, Property 5: SEO metadata completeness
 *
 * Validates: Requirements 7.1, 7.2
 *
 * For any page with title and description frontmatter, the buildSeoMeta function
 * SHALL produce an object containing: a non-empty title incorporating both the page
 * title and site name, a non-empty description, a valid canonical URL, and all Open
 * Graph fields (og:title, og:description, og:image, og:url) plus a twitter:card value.
 */
test.prop(
  [fc.string({ minLength: 1 }), fc.string({ minLength: 1 }), fc.string()],
  { numRuns: 100 }
)(
  'Feature: docs-site, Property 5: SEO metadata completeness',
  (title, description, slug) => {
    const result = buildSeoMeta({ title, description }, slug);

    // Title must contain the site name "CORTEX" and be non-empty
    expect(result.title.toUpperCase()).toContain('CORTEX');
    expect(result.title.length).toBeGreaterThan(0);

    // Description must be non-empty
    expect(result.description.length).toBeGreaterThan(0);

    // Canonical URL must start with the site base URL
    expect(result.canonical).toMatch(new RegExp(`^${siteConfig.url.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}/`));

    // All Open Graph fields must be non-empty
    expect(result.ogTitle.length).toBeGreaterThan(0);
    expect(result.ogDescription.length).toBeGreaterThan(0);
    expect(result.ogImage.length).toBeGreaterThan(0);
    expect(result.ogUrl.length).toBeGreaterThan(0);

    // Twitter card must be summary_large_image
    expect(result.twitterCard).toBe('summary_large_image');
  }
);
