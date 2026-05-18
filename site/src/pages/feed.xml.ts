import rss from '@astrojs/rss';
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
  return rss({
    title: `${siteConfig.name} Docs`,
    description: siteConfig.description,
    site: context.site!.toString(),
    items: docs.map((doc) => ({
      title: doc.title,
      description: doc.description,
      link: `/docs/${doc.slug}/`,
      pubDate: new Date(),
    })),
    customData: `<language>${siteConfig.language}</language>`,
  });
}
