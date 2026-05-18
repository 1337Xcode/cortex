import { siteConfig } from '../../site.config';
import { withBase } from './paths';
import type { ISeoMeta, IPageSeo } from '../types/seo';

export function buildSeoMeta(pageSeo: IPageSeo, slug: string): ISeoMeta {
  const title = pageSeo.title
    ? `${siteConfig.name} / ${pageSeo.title}`
    : siteConfig.name;
  const description = pageSeo.description || siteConfig.description;
  const slugPath = slug ? slug.replace(/^\//, '').replace(/\/$/, '') : '';
  const canonical = slugPath
    ? `${siteConfig.url}/${slugPath}/`
    : `${siteConfig.url}/`;
  const ogImagePath = siteConfig.ogImage.replace(/^\//, '');
  const ogImage = pageSeo.ogImage || `${siteConfig.url}/${ogImagePath}`;

  return {
    title,
    description,
    canonical,
    ogTitle: title,
    ogDescription: description,
    ogImage,
    ogUrl: canonical,
    twitterCard: 'summary_large_image',
    jsonLd: [],
  };
}
