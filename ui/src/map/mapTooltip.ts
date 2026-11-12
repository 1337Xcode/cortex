// What a mark or a link under the pointer says, in the map's own words and with its number: a
// file's lines, reads and commits, a fact badge's count, a folder's file count, a link's shared
// commits or strength, a live session's newest file and the ones before it. Read from what the
// frame drew, so the sentence names exactly the mark the pointer is on.
import type { PickingInfo } from '@deck.gl/core';
import type { LiveSession } from '../api/live';
import { liveSentence, type NamedLiveFile } from '../text/liveWords';
import { badgeSentence, fileSentence, folderSentence, linkSentence } from '../text/vocabulary';
import { fileMark } from './fileEncoding';
import { LAYER_ID, type FrameBuffers } from './layers';
import type { Scene } from './scene';

/** The files of `session` the snapshot holds, newest first, with what the session did to each. */
export function namedLiveFiles(session: LiveSession, scene: Scene): NamedLiveFile[] {
  const { files } = scene.snapshot;
  return session.files.flatMap((entry) => {
    const file = files[entry.file];
    return file === undefined ? [] : [{ path: file.path, mode: entry.mode }];
  });
}

/** The sentence of the hub row `index` of the layer `layerId`, or null. */
function folderPick(
  frame: FrameBuffers,
  scene: Scene,
  layerId: string,
  index: number,
): string | null {
  const buffers =
    layerId === LAYER_ID.hubs
      ? frame.hubs
      : layerId === LAYER_ID.collapsed
        ? frame.collapsed
        : frame.fileDiscs;
  const hub = buffers.ids[index];
  if (hub === undefined) return null;
  return folderSentence(scene.layout.hubPath[hub] ?? '', scene.layout.hubFileCount[hub] ?? 0);
}

/** The sentence of link row `index`, or null. */
function linkPick(frame: FrameBuffers, scene: Scene, index: number): string | null {
  const { files, links } = scene.snapshot;
  const link = links[frame.links.ids[index] ?? -1];
  const from = link === undefined ? undefined : files[link.from];
  const to = link === undefined ? undefined : files[link.to];
  return link === undefined || from === undefined || to === undefined
    ? null
    : linkSentence(link, from, to);
}

/** The sentence of marker row `index`, or null. */
function markerPick(frame: FrameBuffers, scene: Scene, index: number): string | null {
  const session = frame.live.sessions[frame.live.markers.ids[index] ?? -1];
  return session === undefined ? null : liveSentence(session, namedLiveFiles(session, scene));
}

/** The explanation of what was picked, or null for empty canvas. */
export function explainPick(
  info: PickingInfo,
  frame: FrameBuffers,
  scene: Scene,
  nowMs: number,
): string | null {
  if (info.index < 0) return null;
  const { files } = scene.snapshot;
  const layerId = info.layer?.id;
  switch (layerId) {
    case LAYER_ID.files: {
      const file = files[frame.files.ids[info.index] ?? -1];
      return file === undefined ? null : fileSentence(file, fileMark(file, nowMs, scene.hasReads));
    }
    case LAYER_ID.badges: {
      const file = files[frame.badges.ids[info.index] ?? -1];
      return file === undefined ? null : badgeSentence(file);
    }
    case LAYER_ID.links:
      return linkPick(frame, scene, info.index);
    case LAYER_ID.markers:
      return markerPick(frame, scene, info.index);
    case LAYER_ID.hubs:
    case LAYER_ID.collapsed:
    case LAYER_ID.fileDiscs:
      return folderPick(frame, scene, layerId, info.index);
    default:
      return null;
  }
}
