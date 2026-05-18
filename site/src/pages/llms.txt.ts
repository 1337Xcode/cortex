import type { APIContext } from 'astro';
import { siteConfig } from '../../site.config';

export function GET(context: APIContext) {
  const siteUrl = context.site?.toString().replace(/\/$/, '') || siteConfig.url;
  const body = `# ${siteConfig.name} - ${siteConfig.tagline}

## What is ${siteConfig.name}?
${siteConfig.description}

## Installation
npx @1337xcode/cortex install

## Key Features
- 29 language support via tree-sitter
- 32 MCP tools for AI coding agents
- Sub-second repository indexing
- Security vulnerability analysis
- Persistent memory layer for AI agents
- Multi-repository federation

## Links
- Website: ${siteUrl}
- GitHub: ${siteConfig.social.github}
- npm: ${siteConfig.social.npm}
- Documentation: ${siteUrl}/docs/getting-started/
- Issues: ${siteUrl}/issues/
- Visualization: ${siteUrl}/visualization/

## Quick start
cortex index && cortex install && cortex serve
`;
  return new Response(body, { headers: { 'Content-Type': 'text/plain' } });
}
