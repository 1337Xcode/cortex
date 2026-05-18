import { useState } from 'react';
import { Play, X } from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { cn } from '@/lib/utils';

type AnimationStyle = 'from-bottom' | 'from-center' | 'from-top' | 'fade';

interface HeroVideoDialogProps {
  animationStyle?: AnimationStyle;
  videoSrc: string;
  thumbnailSrc: string;
  thumbnailAlt?: string;
  className?: string;
  /** When set, modal shows this message instead of an iframe (placeholder mode). */
  placeholderLabel?: string;
}

const animationVariants = {
  'from-bottom': {
    initial: { y: '100%', opacity: 0 },
    animate: { y: 0, opacity: 1 },
    exit: { y: '100%', opacity: 0 },
  },
  'from-center': {
    initial: { scale: 0.5, opacity: 0 },
    animate: { scale: 1, opacity: 1 },
    exit: { scale: 0.5, opacity: 0 },
  },
  'from-top': {
    initial: { y: '-100%', opacity: 0 },
    animate: { y: 0, opacity: 1 },
    exit: { y: '-100%', opacity: 0 },
  },
  fade: {
    initial: { opacity: 0 },
    animate: { opacity: 1 },
    exit: { opacity: 0 },
  },
};

export function HeroVideoDialog({
  animationStyle = 'from-center',
  videoSrc,
  thumbnailSrc,
  thumbnailAlt = 'Video thumbnail',
  className,
  placeholderLabel,
}: HeroVideoDialogProps) {
  const [isVideoOpen, setIsVideoOpen] = useState(false);
  const selectedAnimation = animationVariants[animationStyle];
  const isPlaceholder = Boolean(placeholderLabel);

  return (
    <div className={cn('relative', className)}>
      <button
        type="button"
        className="group relative w-full overflow-hidden rounded-xl border border-border bg-card shadow-sm"
        onClick={() => setIsVideoOpen(true)}
        aria-label="Play demo video"
      >
        <img
          src={thumbnailSrc}
          alt={thumbnailAlt}
          className="aspect-video w-full object-cover transition-transform duration-300 group-hover:scale-[1.02]"
          loading="lazy"
        />
        <span className="absolute inset-0 flex items-center justify-center bg-black/25 transition-colors group-hover:bg-black/35">
          <span className="flex size-14 items-center justify-center rounded-full bg-background/90 text-foreground shadow-md">
            <Play className="size-6 fill-current ml-0.5" aria-hidden />
          </span>
        </span>
      </button>

      <AnimatePresence>
        {isVideoOpen ? (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            role="dialog"
            aria-modal="true"
            aria-label="Demo video"
            className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-4"
            onClick={() => setIsVideoOpen(false)}
          >
            <motion.div
              initial={selectedAnimation.initial}
              animate={selectedAnimation.animate}
              exit={selectedAnimation.exit}
              transition={{ type: 'spring', damping: 28, stiffness: 260 }}
              className="relative w-full max-w-4xl overflow-hidden rounded-xl bg-card shadow-2xl"
              onClick={(e) => e.stopPropagation()}
            >
              <button
                type="button"
                className="absolute top-3 right-3 z-10 flex size-9 items-center justify-center rounded-full bg-black/60 text-white hover:bg-black/80"
                onClick={() => setIsVideoOpen(false)}
                aria-label="Close video"
              >
                <X className="size-5" aria-hidden />
              </button>
              {isPlaceholder ? (
                <div className="flex aspect-video w-full flex-col items-center justify-center gap-3 bg-muted px-6 text-center">
                  <p className="text-lg font-semibold text-foreground">Cortex demo</p>
                  <p className="max-w-md text-sm text-muted-foreground">{placeholderLabel}</p>
                </div>
              ) : (
                <div className="aspect-video w-full bg-black">
                  <iframe
                    src={videoSrc}
                    title={thumbnailAlt}
                    className="h-full w-full border-0"
                    allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
                    allowFullScreen
                  />
                </div>
              )}
            </motion.div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
