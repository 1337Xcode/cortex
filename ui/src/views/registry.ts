// Every view of the map, in the order of the switch and of their keys (1 to 5). Adding a view is
// one file in this folder and one line here; removing one is deleting both.
import { couplingView } from './coupling';
import { defaultView } from './default';
import { fogView } from './fog';
import { heatView } from './heat';
import { recentView } from './recent';

/** The views, the first being the map's own look. */
export const VIEWS = [defaultView, fogView, heatView, couplingView, recentView] as const;

/** One view's name. */
export type ViewName = (typeof VIEWS)[number]['name'];

/** The view the map opens with. */
export const DEFAULT_VIEW: ViewName = defaultView.name;

/** The view called `name`. */
export function viewNamed(name: ViewName): (typeof VIEWS)[number] {
  return VIEWS.find((view) => view.name === name) ?? defaultView;
}
