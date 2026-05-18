import { siteConfig } from '../../site.config';

export function buildWebSiteSchema() {
  return {
    '@context': 'https://schema.org',
    '@type': 'WebSite',
    name: siteConfig.name,
    url: siteConfig.url,
    description: siteConfig.description,
    author: { '@type': 'Organization', name: siteConfig.author },
  };
}

export function buildSoftwareApplicationSchema() {
  return {
    '@context': 'https://schema.org',
    '@type': 'SoftwareApplication',
    name: siteConfig.name,
    description: siteConfig.description,
    applicationCategory: siteConfig.applicationCategory,
    operatingSystem: siteConfig.operatingSystem,
    programmingLanguage: siteConfig.programmingLanguage,
    url: siteConfig.url,
    offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
    license: 'https://opensource.org/licenses/MIT',
  };
}

export function buildFaqSchema(faqs: { question: string; answer: string }[]) {
  return {
    '@context': 'https://schema.org',
    '@type': 'FAQPage',
    mainEntity: faqs.map((faq) => ({
      '@type': 'Question',
      name: faq.question,
      acceptedAnswer: { '@type': 'Answer', text: faq.answer },
    })),
  };
}

export function buildBreadcrumbSchema(items: { name: string; url: string }[]) {
  return {
    '@context': 'https://schema.org',
    '@type': 'BreadcrumbList',
    itemListElement: items.map((item, index) => ({
      '@type': 'ListItem',
      position: index + 1,
      name: item.name,
      item: item.url,
    })),
  };
}

export const homepageFaqs = [
  {
    question: 'How do I install Cortex?',
    answer: 'Run npx @1337xcode/cortex install, use the install.sh script, or build from source with cargo. Then run cortex index, cortex install, and cortex serve.',
  },
  {
    question: 'Does Cortex work offline?',
    answer: 'Yes. Cortex is a single Rust binary with a local SQLite call graph. No cloud API keys or network calls are required for indexing or MCP tools.',
  },
  {
    question: 'How many languages does Cortex support?',
    answer: 'Cortex supports 29 programming languages via tree-sitter parsers with incremental re-indexing.',
  },
  {
    question: 'What MCP tools does Cortex expose?',
    answer: 'Cortex exposes 32 MCP tools for navigation, graph analysis, security, memory, and federation. Smart-tools mode exposes 5 tools plus an ask meta-tool for routing.',
  },
  {
    question: 'What security features are included?',
    answer: 'Local taint flow analysis, OWASP Top 10 checks, and SBOM generation, all running offline on your machine.',
  },
  {
    question: 'How does Cortex reduce token usage for AI agents?',
    answer: 'Agents query the SQLite call graph over MCP instead of reading raw files, typically using 10-100x fewer tokens for structural questions.',
  },
];
