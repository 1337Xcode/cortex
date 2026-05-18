import { BookOpen } from 'lucide-react';
import { IconBrandGithub, IconBug, IconView360 } from '@tabler/icons-react';
import { IconTooltipLink } from '@/components/ui/icon-tooltip-link';
import { siteConfig } from '../../../site.config';

const BASE = import.meta.env.BASE_URL || '/';
function path(p: string) {
  const normalized = p.startsWith('/') ? p : `/${p}`;
  const basePath = BASE.endsWith('/') ? BASE.slice(0, -1) : BASE;
  if (!basePath || basePath === '/') return normalized;
  return `${basePath}${normalized}`;
}

const links = [
  { label: 'Documentation', href: path('/docs/getting-started/'), icon: BookOpen, lucide: true as const },
  { label: 'Visualization', href: path('/visualization/'), icon: IconView360, tabler: true as const },
  { label: 'Issues', href: path('/issues/'), icon: IconBug, tabler: true as const },
];

export default function HeaderNav() {
  return (
    <nav className="hidden md:flex items-center gap-0.5" aria-label="Main navigation">
      {links.map(({ label, href, icon: Icon, ...rest }) => (
        <IconTooltipLink key={href} href={href} label={label}>
          {'tabler' in rest && rest.tabler ? (
            <Icon size={18} stroke={1.75} aria-hidden />
          ) : (
            <Icon className="size-[18px]" strokeWidth={1.75} aria-hidden />
          )}
        </IconTooltipLink>
      ))}
      <IconTooltipLink href={siteConfig.social.github} label="GitHub" external>
        <IconBrandGithub size={18} stroke={1.75} aria-hidden />
      </IconTooltipLink>
    </nav>
  );
}
