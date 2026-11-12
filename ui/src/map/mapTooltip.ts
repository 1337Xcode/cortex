// What a mark or a link under the pointer says, in the map's own words and with its number: a
// file's lines, reads and commits, a folder's file count, a link's shared commits or strength.
// Read from what the frame drew, so the sentence names exactly the mark the pointer is on.
import type { PickingInfo } from '@deck.gl/core';
import { fileSentence, folderSentence, linkSentence } from '../text/vocabulary';
import { fileMark } from './fileEncoding';
import { LAYER_ID, type FrameBuffers } from './layers';
import type { Scene } from './scene';

/** The explanation of what was picked, or null for empty canvas. */
export function explainPick(
  info: PickingInfo,
  frame: FrameBuffers,
  scene: Scene,
  nowMs: number,
): string | null {
  if (info.index < 0) return null;
  const { files, links } = scene.snapshot;
  switch (info.layer?.id) {
    case LAYER_ID.files: {
      const file = files[frame.files.ids[info.index] ?? -1];
      return file === undefined ? null : fileSentence(file, fileMark(file, nowMs, scene.hasReads));
    }
    case LAYER_ID.links: {
      const link = links[frame.links.ids[info.index] ?? -1];
      const from = link === undefined ? undefined : files[link.from];
      const to = link === undefined ? undefined : files[link.to];
      return link === undefined || from === undefined || to === undefined
        ? null
        : linkSentence(link, from, to);
    }
    case LAYER_ID.hubs:
    case LAYER_ID.collapsed:
    case LAYER_ID.fileDiscs: {
      const buffers =
        info.layer.id === LAYER_ID.hubs
          ? frame.hubs
          : info.layer.id === LAYER_ID.collapsed
            ? frame.collapsed
            : frame.fileDiscs;
      const hub = buffers.ids[info.index];
      if (hub === undefined) return null;
      return folderSentence(scene.layout.hubPath[hub] ?? '', scene.layout.hubFileCount[hub] ?? 0);
    }
    default:
      return null;
  }
}
