import type { APIContext } from 'astro';
import { siteConfig } from '../../site.config';

export function GET(context: APIContext) {
  const siteUrl = context.site?.toString().replace(/\/$/, '') || siteConfig.url;
  const body = `/* TEAM */
Name: ${siteConfig.author}
Role: Creator & Maintainer
Site: ${siteConfig.social.github}
Location: Earth

/* SITE */
Last update: ${new Date().getFullYear()}
Language: ${siteConfig.language}
Standards: HTML5, CSS3, ES2022
Software: Astro, Tailwind CSS, Three.js
Components: React, 3d-force-graph
URL: ${siteUrl}

/* THANKS */
Tree-sitter - Parsing engine
SQLite - Storage layer
Model Context Protocol - AI integration standard
`;
  return new Response(body, { headers: { 'Content-Type': 'text/plain' } });
}
