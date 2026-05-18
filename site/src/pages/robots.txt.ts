import type { APIContext } from 'astro';
import { siteConfig } from '../../site.config';

export function GET(context: APIContext) {
  const siteUrl = context.site?.toString().replace(/\/$/, '') || siteConfig.url;
  const body = `User-agent: *
Allow: /

Sitemap: ${siteUrl}/sitemap-index.xml
`;
  return new Response(body, { headers: { 'Content-Type': 'text/plain' } });
}
