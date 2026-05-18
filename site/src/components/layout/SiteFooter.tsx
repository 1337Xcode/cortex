import { BookOpen, Network } from 'lucide-react';
import {
  IconBrandGithub,
  IconBrandNpm,
  IconBug,
  IconMessages,
  IconRss,
} from '@tabler/icons-react';
import { AtomFeedIcon } from '@/components/icons/AtomFeedIcon';
import { IconTooltipLink } from '@/components/ui/icon-tooltip-link';
import { siteConfig } from '../../../site.config';

const BASE = import.meta.env.BASE_URL || '/';
function path(p: string) {
  const normalized = p.startsWith('/') ? p : `/${p}`;
  const basePath = BASE.endsWith('/') ? BASE.slice(0, -1) : BASE;
  if (!basePath || basePath === '/') return normalized;
  return `${basePath}${normalized}`;
}

const repo = '1337Xcode/cortex';

type FooterLink = {
  label: string;
  href: string;
  external?: boolean;
  tabler?: boolean;
  lucide?: boolean;
  custom?: 'atom';
  icon: React.ComponentType<{ className?: string; strokeWidth?: number; size?: number; stroke?: number }>;
};

const siteLinks: FooterLink[] = [
  { label: 'Documentation', href: path('/docs/getting-started/'), icon: BookOpen, lucide: true },
  { label: 'Visualization', href: path('/visualization/'), icon: Network, lucide: true },
  { label: 'Issues', href: path('/issues/'), icon: IconBug, tabler: true },
];

const communityLinks: FooterLink[] = [
  { label: 'GitHub', href: siteConfig.social.github, icon: IconBrandGithub, external: true, tabler: true },
  { label: 'Discussions', href: `https://github.com/${repo}/discussions`, icon: IconMessages, external: true, tabler: true },
  { label: 'npm', href: siteConfig.social.npm, icon: IconBrandNpm, external: true, tabler: true },
];

const feedLinks: FooterLink[] = [
  { label: 'RSS feed', href: path('/feed.xml'), icon: IconRss, tabler: true },
  { label: 'Atom feed', href: path('/atom.xml'), icon: IconRss, custom: 'atom' as const },
];

function FooterIcon({ link }: { link: FooterLink }) {
  if (link.custom === 'atom') {
    return <AtomFeedIcon className="size-[18px]" />;
  }
  const Icon = link.icon;
  if (link.tabler) {
    return <Icon size={18} stroke={1.75} aria-hidden />;
  }
  return <Icon className="size-[18px]" strokeWidth={1.75} aria-hidden />;
}

function LinkGroup({ title, links }: { title: string; links: FooterLink[] }) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">{title}</p>
      <div className="flex items-center gap-0.5">
        {links.map((link) => (
          <IconTooltipLink key={link.label} href={link.href} label={link.label} external={link.external}>
            <FooterIcon link={link} />
          </IconTooltipLink>
        ))}
      </div>
    </div>
  );
}

export default function SiteFooter() {
  const homeHref = path('/');

  return (
    <footer className="mt-auto w-full border-t border-border bg-surface-raised/80">
      <div className="mx-auto flex max-w-7xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
        <div className="flex flex-col gap-6 sm:flex-row sm:items-start sm:justify-between">
          <p className="text-sm text-muted-foreground flex flex-wrap items-center gap-x-2">
            <a
              href={homeHref}
              className="font-cortex-pixel text-[10px] text-foreground hover:text-foreground/80 transition-colors"
            >
              {siteConfig.name}
            </a>
            <span className="opacity-30" aria-hidden="true">
              ·
            </span>
            <a
              href="https://github.com/1337XCode"
              target="_blank"
              rel="noopener noreferrer"
              className="rainbow-text font-semibold hover:opacity-90 transition-opacity"
            >
              1337XCode
            </a>
          </p>

          <nav className="flex flex-wrap gap-8 sm:gap-10" aria-label="Footer">
            <LinkGroup title="Site" links={siteLinks} />
            <LinkGroup title="Community" links={communityLinks} />
            <LinkGroup title="Feeds" links={feedLinks} />
          </nav>
        </div>
      </div>
    </footer>
  );
}
