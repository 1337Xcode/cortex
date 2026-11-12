// A brand or language icon as the map draws it: one icon of `@thesvg/react`, in one of its
// variants, filled with the text's own colour (`currentColor`) so the tokens decide its colour and
// its contrast. The two tables (agentIcons.ts, fileIcons.ts) are built from these.
import { createElement, type ComponentType, type FunctionComponent, type SVGProps } from 'react';

/** A thesvg icon component whose `variant` prop takes `V` among its values. */
type ThesvgIcon<V extends string> = ComponentType<SVGProps<SVGSVGElement> & { variant?: V }>;

/** An icon the map can draw beside a name. */
export interface BrandIcon {
  /** The icon's name in thesvg, the end of its import path (`@thesvg/react/claude-code`). */
  readonly name: string;
  /** The variant drawn: one whose shapes have no colour of their own (`mono` where it has one). */
  readonly variant: string;
  /** The icon as an SVG element in the text's colour, sized by its container's stylesheet. */
  readonly Svg: FunctionComponent<SVGProps<SVGSVGElement>>;
}

/**
 * The icon `Icon`, thesvg's `name`, drawn in its `variant` with its root filled with the text's
 * colour. A variant with shapes of their own colours stays coloured, so the tables' tests check
 * that every icon they hold renders in `currentColor` alone.
 */
export function brandIcon<V extends string>(
  name: string,
  Icon: ThesvgIcon<V>,
  variant: V,
): BrandIcon {
  const Svg: FunctionComponent<SVGProps<SVGSVGElement>> = (props) =>
    createElement(Icon, { ...props, variant, fill: 'currentColor' });
  Svg.displayName = `BrandIcon(${name})`;
  return { name, variant, Svg };
}
