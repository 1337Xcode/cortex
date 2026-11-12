// Checks the icon tables' tests share: an icon draws in the text's colour alone, and it is the
// thesvg icon its name says, read from that icon's own module rather than from the table.
import { createElement, type ComponentType, type SVGProps } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect } from 'vitest';
import type { BrandIcon } from './icon';

/** The SVG an icon of a table draws, as markup. */
export function markupOf(icon: BrandIcon): string {
  return renderToStaticMarkup(createElement(icon.Svg));
}

/** The text of the `<title>` the icon carries, or null when it carries none. */
export function titleOf(icon: BrandIcon): string | null {
  return /<title>([^<]*)<\/title>/.exec(markupOf(icon))?.[1] ?? null;
}

/**
 * Expects the icon to be drawn in the text's colour alone: every fill, stroke and stop colour is
 * `currentColor` or `none`, and it holds no inline style, id (two copies on a page would clash),
 * reference to another element, or image.
 */
export function expectDrawnInTextColour(icon: BrandIcon): void {
  const markup = markupOf(icon);
  const colours = [...markup.matchAll(/\b(?:fill|stroke|stop-color|color)="([^"]*)"/g)].map(
    (match) => match[1],
  );
  expect(colours.length, icon.name).toBeGreaterThan(0);
  for (const colour of colours) expect(['currentColor', 'none'], icon.name).toContain(colour);
  expect(markup, icon.name).not.toMatch(/\b(?:style|id|href|xlink:href)=|url\(|<image/);
}

/** Whether `value` is a component React can draw. */
function isComponent(
  value: unknown,
): value is ComponentType<SVGProps<SVGSVGElement> & { variant: string }> {
  return typeof value === 'function' || (typeof value === 'object' && value !== null);
}

/**
 * Expects the icon to draw exactly what thesvg's module of its name draws in its variant, filled
 * with the text's colour: the table's name and the component it imported are the same icon.
 */
export async function expectIsThesvgIcon(icon: BrandIcon): Promise<void> {
  const module: unknown = await import(`@thesvg/react/${icon.name}`);
  const component: unknown =
    typeof module === 'object' && module !== null && 'default' in module ? module.default : null;
  if (!isComponent(component)) throw new Error(`@thesvg/react/${icon.name} exports no icon`);
  const expected = renderToStaticMarkup(
    createElement(component, { variant: icon.variant, fill: 'currentColor' }),
  );
  expect(markupOf(icon), icon.name).toBe(expected);
}
