// Joins class names for the components, letting a later Tailwind class override an earlier one of
// the same kind. tailwind-merge is told the map's own scale names (the spacing, type, size and
// radius steps the stylesheet defines from the tokens), so `text-small` is known as a type size and
// not mistaken for a colour, and `p-sm` replaces `p-md`.
import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';
import { SIZE, SPACE, TYPE } from '../theme/tokens';

/** A camelCase token name in kebab case, as the stylesheet names it. */
function kebab(name: string): string {
  return name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
}

const merge = extendTailwindMerge({
  extend: {
    theme: {
      spacing: [...Object.keys(SPACE), ...Object.keys(SIZE).map(kebab)],
      text: Object.keys(TYPE),
      radius: ['control', 'small'],
    },
  },
});

/** The classes of `inputs`, joined, with conflicting Tailwind classes resolved in favour of the last. */
export function cn(...inputs: ClassValue[]): string {
  return merge(clsx(inputs));
}
