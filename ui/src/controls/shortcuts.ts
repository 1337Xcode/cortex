// The keyboard shortcuts, in one table: each key and what it does. Keys typed into a text field
// belong to the field, except Escape; a key held with Ctrl, Alt or Meta is left to the browser.

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
