import { useEffect, useState } from 'react';
import { DiaTextReveal } from '@/components/ui/dia-text-reveal';
import { siteConfig } from '../../../site.config';

const LINE_CLASS =
  'text-3xl sm:text-5xl md:text-6xl font-bold tracking-tight leading-[1.15] text-text';

const PHRASES = siteConfig.heroHeadlines;

export default function HeroHeadline() {
  const [index, setIndex] = useState(0);

  useEffect(() => {
    const id = window.setInterval(() => {
      setIndex((i) => (i + 1) % PHRASES.length);
    }, 5000);
    return () => window.clearInterval(id);
  }, []);

  const phrase = PHRASES[index];

  return (
    <h1 className="mx-auto flex w-full max-w-4xl flex-col items-center px-4 text-center">
      <span className="sr-only">
        {phrase.top} {phrase.bottom}
      </span>
      <span className={`block w-full ${LINE_CLASS}`} aria-hidden="true">
        {phrase.top}
      </span>
      <span className={`mt-1 flex w-full justify-center ${LINE_CLASS}`} aria-hidden="true">
        <DiaTextReveal
          key={`${index}-${phrase.bottom}`}
          text={phrase.bottom}
          textColor="var(--color-text)"
          className={`inline-block text-center ${LINE_CLASS}`}
          startOnView
          once={false}
          duration={1.2}
        />
      </span>
    </h1>
  );
}
