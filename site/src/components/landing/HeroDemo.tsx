import { HeroVideoDialog } from '@/components/ui/hero-video-dialog';
import { withBase } from '@/lib/paths';

const ENV_VIDEO_ID = (import.meta.env.PUBLIC_DEMO_VIDEO_ID ?? '').trim();
const isPlaceholder = !ENV_VIDEO_ID;

export default function HeroDemo() {
  return (
    <div className="mx-auto mt-10 w-full max-w-3xl px-2">
      <HeroVideoDialog
        animationStyle="from-center"
        videoSrc={isPlaceholder ? '' : `https://www.youtube.com/embed/${ENV_VIDEO_ID}?autoplay=1`}
        thumbnailSrc={
          isPlaceholder
            ? withBase('/demo-poster.svg')
            : `https://img.youtube.com/vi/${ENV_VIDEO_ID}/hqdefault.jpg`
        }
        thumbnailAlt={isPlaceholder ? 'Cortex product demo (preview)' : 'Play Cortex product demo'}
        placeholderLabel={
          isPlaceholder
            ? 'Product demo coming soon. Set PUBLIC_DEMO_VIDEO_ID in your .env to embed a YouTube walkthrough.'
            : undefined
        }
      />
    </div>
  );
}
