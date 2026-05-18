import { useEffect, useRef } from 'react';

const NODES = [
  { x: 0.22, y: 0.35, r: 5, color: '#7c8aff', delay: 0 },
  { x: 0.48, y: 0.28, r: 7, color: '#f78166', delay: 0.4 },
  { x: 0.72, y: 0.42, r: 5, color: '#7ee787', delay: 0.8 },
  { x: 0.35, y: 0.62, r: 4, color: '#d2a8ff', delay: 1.2 },
  { x: 0.58, y: 0.68, r: 6, color: '#79c0ff', delay: 0.6 },
  { x: 0.78, y: 0.58, r: 4, color: '#ffa657', delay: 1 },
];

const EDGES: [number, number][] = [
  [0, 1],
  [1, 2],
  [0, 3],
  [3, 4],
  [4, 2],
  [1, 4],
];

const BG = {
  light: '#e4e2df',
  dark: '#2a2a2e',
};

export function BentoGraphAnimation() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const themeRef = useRef<'light' | 'dark'>('light');

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let frame = 0;
    let raf = 0;

    const draw = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = canvas.getBoundingClientRect();
      const w = Math.max(rect.width, 1);
      const h = Math.max(rect.height, 1);
      if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
        canvas.width = w * dpr;
        canvas.height = h * dpr;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      }

      const dark = themeRef.current === 'dark';
      ctx.fillStyle = dark ? BG.dark : BG.light;
      ctx.fillRect(0, 0, w, h);

      const t = frame * 0.012;
      const positions = NODES.map((n, i) => ({
        x: (n.x + Math.sin(t + n.delay + i) * 0.018) * w,
        y: (n.y + Math.cos(t * 0.9 + n.delay) * 0.015) * h,
        r: n.r,
        color: n.color,
      }));

      ctx.lineWidth = 1;
      for (const [a, b] of EDGES) {
        const pulse = 0.15 + 0.12 * Math.sin(t * 2 + a);
        ctx.strokeStyle = dark
          ? `rgba(168, 177, 255, ${pulse})`
          : `rgba(80, 90, 140, ${pulse + 0.1})`;
        ctx.beginPath();
        ctx.moveTo(positions[a].x, positions[a].y);
        ctx.lineTo(positions[b].x, positions[b].y);
        ctx.stroke();
      }

      for (const p of positions) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r + 4, 0, Math.PI * 2);
        ctx.fillStyle = `${p.color}40`;
        ctx.fill();
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
        ctx.fillStyle = p.color;
        ctx.fill();
      }

      frame += 1;
      raf = requestAnimationFrame(draw);
    };

    const ro = new ResizeObserver(() => draw());
    ro.observe(canvas);

    const getTheme = () =>
      document.documentElement.classList.contains('dark') ? 'dark' : 'light';

    themeRef.current = getTheme();

    const themeObs = new MutationObserver(() => {
      themeRef.current = getTheme();
    });
    themeObs.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });

    raf = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      themeObs.disconnect();
    };
  }, []);

  return (
    <div className="mt-1 flex min-h-[7rem] flex-1 flex-col overflow-hidden rounded-lg border border-border bg-[#e4e2df] shadow-inner dark:bg-[#2a2a2e]">
      <canvas ref={canvasRef} className="h-full min-h-[7rem] w-full" aria-hidden />
    </div>
  );
}
