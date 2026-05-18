/** Atom syndication format logo (orbital rings), not the Atom editor mark. */
export function AtomFeedIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      aria-hidden
    >
      <ellipse cx="12" cy="12" rx="9" ry="3.25" transform="rotate(-35 12 12)" />
      <ellipse cx="12" cy="12" rx="9" ry="3.25" transform="rotate(35 12 12)" />
      <ellipse cx="12" cy="12" rx="9" ry="3.25" transform="rotate(90 12 12)" />
      <circle cx="12" cy="12" r="1.75" fill="currentColor" stroke="none" />
    </svg>
  );
}
