import { RainbowButton } from '@/components/ui/rainbow-button';

type Props = {
  docsHref: string;
};

export default function HeroCta({ docsHref }: Props) {
  function scrollToInstall() {
    document.getElementById('install')?.scrollIntoView({ behavior: 'smooth' });
  }

  const ctaClass =
    'w-full min-w-0 sm:min-w-[14rem] sm:flex-1 sm:max-w-[16rem] px-10 text-base font-semibold whitespace-nowrap';

  return (
    <div className="mt-8 flex w-full max-w-xl flex-col items-stretch justify-center gap-3 px-2 sm:flex-row sm:items-center sm:justify-center">
      <RainbowButton
        variant="default"
        type="button"
        size="lg"
        className={ctaClass}
        onClick={scrollToInstall}
      >
        Install Cortex
      </RainbowButton>
      <a
        href={docsHref}
        className={`inline-flex h-11 items-center justify-center rounded-xl border border-border bg-card text-foreground transition-colors hover:bg-muted ${ctaClass}`}
      >
        View documentation
      </a>
    </div>
  );
}
