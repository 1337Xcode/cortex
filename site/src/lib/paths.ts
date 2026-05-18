/** Astro base URL (e.g. `/cortex/` on GitHub Pages, `/` locally). */
export const base = import.meta.env.BASE_URL;

/** Prefix an internal path with the site base. */
export function withBase(path: string): string {
  const normalized = path.startsWith('/') ? path : `/${path}`;
  const basePath = base.endsWith('/') ? base.slice(0, -1) : base;
  if (!basePath || basePath === '/') return normalized;
  return `${basePath}${normalized}`;
}

/** Build a canonical URL path segment (no trailing slash on slug). */
export function canonicalPath(slug: string): string {
  const path = slug ? `/${slug.replace(/^\//, '').replace(/\/$/, '')}` : '';
  return withBase(path || '/');
}
