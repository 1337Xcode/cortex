import type { APIContext } from 'astro';
import { siteConfig } from '../../site.config';

const docs = [
  { title: 'Getting Started', slug: 'getting-started', description: 'Install Cortex and index your first repository in under a minute.' },
  { title: 'Architecture', slug: 'architecture', description: 'How Cortex works internally: tree-sitter parsing, SQLite storage, and MCP server.' },
  { title: 'CLI Reference', slug: 'cli-reference', description: 'Complete reference for all Cortex CLI commands and options.' },
  { title: 'Configuration', slug: 'configuration', description: 'Configure Cortex behavior with cortex.toml and environment variables.' },
  { title: 'IDE Setup', slug: 'ide-setup', description: 'Set up Cortex with VS Code, Cursor, Windsurf, and other editors.' },
  { title: 'MCP Tools', slug: 'tools', description: 'Reference for all 32 MCP tools exposed by the Cortex server.' },
];

export function GET(context: APIContext) {
  const site = context.site!.toString().replace(/\/$/, '');
  const updated = new Date().toISOString();

  const entries = docs.map((doc) => `
    <entry>
      <title>${doc.title}</title>
      <link href="${site}/docs/${doc.slug}/" rel="alternate" />
      <id>${site}/docs/${doc.slug}/</id>
      <updated>${updated}</updated>
      <summary>${doc.description}</summary>
    </entry>`).join('');

  const atom = `<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>${siteConfig.name} Docs</title>
  <subtitle>${siteConfig.description}</subtitle>
  <link href="${site}/atom.xml" rel="self" />
  <link href="${site}" />
  <id>${site}/</id>
  <updated>${updated}</updated>
  <author>
    <name>${siteConfig.author}</name>
  </author>${entries}
</feed>`;

  return new Response(atom.trim(), {
    headers: { 'Content-Type': 'application/atom+xml; charset=utf-8' },
  });
}
