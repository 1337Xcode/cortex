import { useCallback, useEffect, useRef, useState } from 'react';
import { cortexEmbedding3, formatEmbedding3 } from '@/lib/cortex-embedding';
import { cn } from '@/lib/utils';

const LABEL = 'CORTEX';
const TARGET = cortexEmbedding3(LABEL);
const TARGET_STR = formatEmbedding3(TARGET);
const CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789@#$%&*';

const PIXEL_TEXT = 'font-cortex-pixel leading-none whitespace-nowrap';
const LABEL_CLASS = `${PIXEL_TEXT} text-[10px] sm:text-[11px]`;
const EMBED_CLASS = `${PIXEL_TEXT} text-[8px] sm:text-[9px]`;

type Props = {
  href: string;
  className?: string;
};

export default function CortexLogo({ href, className }: Props) {
  const [displayText, setDisplayText] = useState(LABEL);
  const [phase, setPhase] = useState<'label' | 'scramble' | 'embedding'>('label');
  const rafRef = useRef<number | null>(null);
  const hoverRef = useRef(false);

  const clearAnim = useCallback(() => {
    if (rafRef.current !== null) {
      window.clearInterval(rafRef.current);
      rafRef.current = null;
    }
  }, []);

  const scrambleToEmbedding = useCallback(() => {
    clearAnim();
    setPhase('scramble');
    let step = 0;
    const totalSteps = 14;
    const target = TARGET_STR;

    rafRef.current = window.setInterval(() => {
      step++;
      if (step >= totalSteps) {
        clearAnim();
        setDisplayText(target);
        setPhase('embedding');
        return;
      }
      // Each step reveals more of the target, rest is random
      const revealed = Math.floor((step / totalSteps) * target.length);
      let result = '';
      for (let i = 0; i < target.length; i++) {
        if (i < revealed) {
          result += target[i];
        } else if (target[i] === ' ' || target[i] === '[' || target[i] === ']' || target[i] === ',') {
          result += target[i]; // Keep structural chars
        } else {
          result += CHARS[Math.floor(Math.random() * CHARS.length)];
        }
      }
      setDisplayText(result);
    }, 35);
  }, [clearAnim]);

  const scrambleToLabel = useCallback(() => {
    clearAnim();
    setPhase('scramble');
    let step = 0;
    const totalSteps = 10;

    rafRef.current = window.setInterval(() => {
      step++;
      if (step >= totalSteps) {
        clearAnim();
        setDisplayText(LABEL);
        setPhase('label');
        return;
      }
      // Scramble back to CORTEX letter by letter
      const revealed = Math.floor((step / totalSteps) * LABEL.length);
      let result = '';
      for (let i = 0; i < LABEL.length; i++) {
        if (i < revealed) {
          result += LABEL[i];
        } else {
          result += CHARS[Math.floor(Math.random() * CHARS.length)];
        }
      }
      setDisplayText(result);
    }, 40);
  }, [clearAnim]);

  const onEnter = useCallback(() => {
    hoverRef.current = true;
    scrambleToEmbedding();
  }, [scrambleToEmbedding]);

  const onLeave = useCallback(() => {
    hoverRef.current = false;
    scrambleToLabel();
  }, [scrambleToLabel]);

  useEffect(() => () => clearAnim(), [clearAnim]);

  const isEmbed = phase === 'embedding' || (phase === 'scramble' && hoverRef.current);

  return (
    <a
      href={href}
      className={cn(
        'relative inline-flex h-[18px] items-center shrink-0 text-foreground transition-opacity hover:opacity-90',
        isEmbed
          ? 'min-w-[10.5rem] sm:min-w-[11.5rem] overflow-visible'
          : 'min-w-[8.75rem] sm:min-w-[9.25rem] overflow-hidden',
        className,
      )}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      onFocus={onEnter}
      onBlur={onLeave}
      aria-label="Cortex home"
    >
      <span
        className={cn(
          isEmbed ? EMBED_CLASS : LABEL_CLASS,
          'flex items-center transition-all duration-100 ease-out',
        )}
      >
        {displayText}
      </span>
    </a>
  );
}
