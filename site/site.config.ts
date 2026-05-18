import type { ISiteConfig } from './src/types/site-config';

export const siteConfig: ISiteConfig = {
  name: 'CORTEX',
  tagline: 'One binary. Zero dependencies. Local code intelligence for AI coding agents.',
  description:
    'Cortex builds a local call graph of your repository and exposes it over MCP for AI assistants.',
  heroHeadlines: [
    { top: 'One binary.', bottom: 'Zero dependencies.' },
    { top: 'Local code intelligence', bottom: 'for AI coding agents.' },
    { top: 'Index your repository.', bottom: 'Query it over MCP.' },
  ],
  url: 'https://1337xcode.github.io/cortex',
  ogImage: '/og-image.png',
  repository: 'https://github.com/1337Xcode/cortex',
  social: {
    github: 'https://github.com/1337Xcode/cortex',
    npm: 'https://www.npmjs.com/package/@1337xcode/cortex',
  },
  author: '1337XCode',
  language: 'en',
  themeColor: '#1f1f1f',
  version: '1.0.0',
  keywords: ['MCP', 'code intelligence', 'call graph', 'tree-sitter', 'AI coding', 'static analysis'],
  applicationCategory: 'DeveloperApplication',
  operatingSystem: 'Linux, macOS, Windows',
  programmingLanguage: 'Rust',
};
