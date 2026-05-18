export interface HeroHeadlinePhrase {
  top: string;
  bottom: string;
}

export interface ISiteConfig {
  name: string;
  tagline: string;
  description: string;
  heroHeadlines: HeroHeadlinePhrase[];
  url: string;
  ogImage: string;
  repository: string;
  social: {
    github: string;
    npm: string;
  };
  author: string;
  language: string;
  themeColor: string;
  version: string;
  keywords: string[];
  // AEO/GEO defaults
  applicationCategory: string;
  operatingSystem: string;
  programmingLanguage: string;
}
