import confetti from 'canvas-confetti';

export function fireConfettiFromElement(el: HTMLElement, particleCount = 80) {
  const rect = el.getBoundingClientRect();
  const x = (rect.left + rect.width / 2) / window.innerWidth;
  const y = (rect.top + rect.height / 2) / window.innerHeight;

  void confetti({
    particleCount,
    spread: 80,
    startVelocity: 32,
    origin: { x, y },
    disableForReducedMotion: true,
    zIndex: 9999,
  });

  window.setTimeout(() => {
    void confetti({
      particleCount: Math.round(particleCount * 0.4),
      spread: 55,
      startVelocity: 22,
      origin: { x, y: Math.min(y + 0.05, 1) },
      disableForReducedMotion: true,
      zIndex: 9999,
    });
  }, 120);
}
