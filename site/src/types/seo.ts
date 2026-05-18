export interface ISeoMeta {
  title: string;
  description: string;
  canonical: string;
  ogTitle: string;
  ogDescription: string;
  ogImage: string;
  ogUrl: string;
  twitterCard: 'summary' | 'summary_large_image';
  jsonLd: Record<string, unknown>[];
}

export interface IPageSeo {
  title?: string;
  description?: string;
  ogImage?: string;
}
