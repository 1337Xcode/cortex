import { useEffect } from 'react';

type Node = { x: number; y: number; id: number };

const INTERACTIVE =
  'a,button,input,textarea,select,option,label,summary,[role="button"],[role="tab"],[role="link"],[contenteditable="true"]';

function isInteractive(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return true;
  return Boolean(target.closest(INTERACTIVE));
}

export default function ClickGraphEffect() {
  useEffect(() => {
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (reduced) return;

    const layer = document.createElement("div");
    layer.id = 'click-graph-layer';
    layer.setAttribute('aria-hidden', 'true');
    layer.style.cssText =
      'position:fixed;inset:0;pointer-events:none;z-index:9999;overflow:hidden;';
    document.body.appendChild(layer);

    function spawn(clientX: number, clientY: number) {
      const count = 3 + Math.floor(Math.random() * 3);
      const nodes: Node[] = Array.from({ length: count }, () => ({
        id: Math.floor(Math.random() * 900) + 100,
        x: clientX + (Math.random() - 0.5) * 72,
        y: clientY + (Math.random() - 0.5) * 56,
      }));
      const edges: [number, number][] = [];
      for (let i = 0; i < count - 1; i++) edges.push([i, i + 1]);
      if (count > 2) edges.push([0, count - 1]);

      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.setAttribute('class', 'click-graph-burst');
      svg.style.cssText = 'position:absolute;left:0;top:0;width:100%;height:100%;';

      const g = document.createElementNS('http://www.w3.org/2000/svg', 'g');
      g.setAttribute('opacity', '0.85');

      edges.forEach(([a, b]) => {
        const line = document.createElementNS('http://www.w3.org/2000/svg', 'line');
        line.setAttribute('x1', String(nodes[a].x));
        line.setAttribute('y1', String(nodes[a].y));
        line.setAttribute('x2', String(nodes[b].x));
        line.setAttribute('y2', String(nodes[b].y));
        line.setAttribute('stroke', 'currentColor');
        line.setAttribute('stroke-width', '1');
        line.setAttribute('class', 'text-foreground/25');
        g.appendChild(line);
      });

      nodes.forEach((n) => {
        const circle = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
        circle.setAttribute('cx', String(n.x));
        circle.setAttribute('cy', String(n.y));
        circle.setAttribute('r', '3');
        circle.setAttribute('fill', 'currentColor');
        circle.setAttribute('class', 'text-foreground/50');
        g.appendChild(circle);

        const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        text.setAttribute('x', String(n.x + 6));
        text.setAttribute('y', String(n.y - 6));
        text.setAttribute('font-size', '9');
        text.setAttribute('font-family', 'IBM Plex Mono, monospace');
        text.setAttribute('fill', 'currentColor');
        text.setAttribute('class', 'text-foreground/60');
        text.textContent = String(n.id);
        g.appendChild(text);
      });

      svg.appendChild(g);
      layer.appendChild(svg);

      requestAnimationFrame(() => {
        g.style.transition = 'opacity 700ms ease, transform 700ms ease';
        g.style.transform = 'scale(1.08)';
        g.style.opacity = '0';
      });

      window.setTimeout(() => svg.remove(), 750);
    }

    function onClick(e: MouseEvent) {
      if (e.button !== 0 || isInteractive(e.target)) return;
      spawn(e.clientX, e.clientY);
    }

    document.addEventListener('click', onClick, { passive: true });
    return () => {
      document.removeEventListener('click', onClick);
      layer.remove();
    };
  }, []);

  return null;
}
