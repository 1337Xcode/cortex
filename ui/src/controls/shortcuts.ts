// The keyboard shortcuts, in one table: each key and what it does, and the digits that pick a
// view. Keys typed into a text field belong to the field, except Escape; a key held with Ctrl, Alt
// or Meta is left to the browser.

/** What a shortcut does. */
export type ShortcutAction =
  | 'focusSearch'
  | 'fit'
  | 'center'
  | 'zoomIn'
  | 'zoomOut'
  | 'panLeft'
  | 'panRight'
  | 'panUp'
  | 'panDown'
  | 'toggleLegend'
  | 'clear';

/** The keys of `KeyboardEvent.key` and the action each starts. */
const SHORTCUTS: ReadonlyMap<string, ShortcutAction> = new Map([
  ['/', 'focusSearch'],
  ['z', 'fit'],
  ['Z', 'fit'],
  ['c', 'center'],
  ['C', 'center'],
  ['+', 'zoomIn'],
  ['=', 'zoomIn'],
  ['-', 'zoomOut'],
  ['_', 'zoomOut'],
  ['ArrowLeft', 'panLeft'],
  ['ArrowRight', 'panRight'],
  ['ArrowUp', 'panUp'],
  ['ArrowDown', 'panDown'],
  ['l', 'toggleLegend'],
  ['L', 'toggleLegend'],
  ['Escape', 'clear'],
]);

/** The parts of a key event the table needs. */
export interface KeyInput {
  readonly key: string;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly metaKey: boolean;
  /** Whether the key went to a text field, a slider or another control that uses keys itself. */
  readonly isInField: boolean;
}

/** The action for a key, or null when the key is not a shortcut where it was pressed. */
export function shortcutFor(input: KeyInput): ShortcutAction | null {
  if (input.ctrlKey || input.altKey || input.metaKey) return null;
  const action = SHORTCUTS.get(input.key) ?? null;
  if (action === null) return null;
  return input.isInField && action !== 'clear' ? null : action;
}

/**
 * The view a key picks: `1` the first, `2` the second, up to `count`; null for any other key, a
 * key held with Ctrl, Alt or Meta, or a key typed into a field.
 */
export function viewIndexFor(input: KeyInput, count: number): number | null {
  if (input.ctrlKey || input.altKey || input.metaKey || input.isInField) return null;
  if (!/^[1-9]$/.test(input.key)) return null;
  const index = Number(input.key) - 1;
  return index < count ? index : null;
}

/** A panel section a letter opens and moves the focus to, wherever the focus is. */
export type SectionKeyName =
  'activity' | 'warnings' | 'receipts' | 'facts' | 'history' | 'told' | 'symbols';

/**
 * The letter of each section a person opens often but that sits closed or far down its panel: in
 * the overview the activity, the warnings and the receipts; in a file's panel its facts, the
 * commits behind its lines, what Cortex told agents and its symbols. One letter each, none shared
 * with the map's own keys above.
 */
export const SECTION_KEYS: Readonly<Record<SectionKeyName, string>> = {
  activity: 'a',
  warnings: 'w',
  receipts: 'r',
  facts: 'f',
  history: 'h',
  told: 't',
  symbols: 's',
};

/** Every section a letter opens, in the order of `SECTION_KEYS`. */
const SECTION_NAMES: readonly SectionKeyName[] = [
  'activity',
  'warnings',
  'receipts',
  'facts',
  'history',
  'told',
  'symbols',
];

/**
 * The section a key opens, or null: a key held with Ctrl, Alt or Meta, or typed into a field, is
 * never one. Upper and lower case open the same section.
 */
export function sectionFor(input: KeyInput): SectionKeyName | null {
  if (input.ctrlKey || input.altKey || input.metaKey || input.isInField) return null;
  const letter = input.key.toLowerCase();
  return SECTION_NAMES.find((name) => SECTION_KEYS[name] === letter) ?? null;
}

/** Whether an event target is a control that uses typed or arrow keys itself. */
export function isKeyField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  );
}
