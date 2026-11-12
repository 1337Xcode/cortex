// A view applied to one snapshot: the files' tones and, for a view that tints folders, each
// folder's strongest tone. Built once when the view, the snapshot or "now" changes; every frame's
// encoders read from it.
import type { Scene } from '../map/scene';
import { folderTonesOf, type FolderTones } from './folderTones';
import type { MapViewDefinition, ViewEncoding } from './view';

/** What the encoders draw a view from. */
export interface ViewPaint {
  /** The view's title, as the switch and the legend name it. */
  readonly title: string;
  readonly encoding: ViewEncoding;
  /** Each folder's strongest tone, or null when the view does not tint. */
  readonly folders: FolderTones | null;
}

/** Applies `view` to `scene`; "the last 24 hours" ends at `nowMs`. */
export function paintView(view: MapViewDefinition, scene: Scene, nowMs: number): ViewPaint {
  const encoding = view.encode(scene, nowMs);
  const folders = encoding.isTinted ? folderTonesOf(scene, encoding) : null;
  return { title: view.title, encoding, folders };
}
