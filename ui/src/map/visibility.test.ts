// Tests for what each level of detail draws, on the seven-file tree. Hub states come from the
// level-of-detail tests (radii 120, 15, 50, 15, 15 for root, a, b, b/deep, c); the file stand-ins
// follow from which hub each file sits in.
import { describe, expect, it } from 'vitest';
import { levelOfDetail } from '../layout/lod';
import { sevenFileScene } from './testScene';
import { computeVisibility } from './visibility';

const { layout } = sevenFileScene();

describe('computeVisibility', () => {
  it('draws every file at zoom 1, where every hub is open', () => {
    const visibility = computeVisibility(layout, levelOfDetail(layout, 1), true);
    expect(Array.from(visibility.openHubs)).toEqual([0, 1, 2, 3, 4]);
    expect(Array.from(visibility.collapsedHubs)).toEqual([]);
    expect(Array.from(visibility.visibleFiles)).toEqual([0, 1, 2, 3, 4, 5, 6]);
    expect(Array.from(visibility.fileStandIn)).toEqual([-1, -1, -1, -1, -1, -1, -1]);
  });

  it('stands the files of a, b/deep and c in for their collapsed hubs at zoom 0', () => {
    // Open: root, b. Collapsed: a (files 1, 2), b/deep (file 3), c (file 6).
    const visibility = computeVisibility(layout, levelOfDetail(layout, 0), true);
    expect(Array.from(visibility.openHubs)).toEqual([0, 2]);
    expect(Array.from(visibility.collapsedHubs)).toEqual([1, 3, 4]);
    expect(Array.from(visibility.visibleFiles)).toEqual([0, 4, 5]);
    expect(Array.from(visibility.fileStandIn)).toEqual([-1, 1, 1, 3, -1, -1, 4]);
    expect(Array.from(visibility.hubStandIn)).toEqual([-1, 1, -1, 3, 4]);
  });

  it('hands a hidden hub to its collapsed ancestor at zoom -2', () => {
    // b collapses and b/deep is hidden inside it, so b/deep/leaf.rs stands in as b (hub 2).
    const visibility = computeVisibility(layout, levelOfDetail(layout, -2), true);
    expect(Array.from(visibility.hubStandIn)).toEqual([-1, 1, 2, 2, 4]);
    expect(Array.from(visibility.fileStandIn)).toEqual([-1, 1, 1, 2, 2, 2, 4]);
    expect(Array.from(visibility.visibleFiles)).toEqual([0]);
  });

  it('draws no single file while the rings are too close, each standing in as its own hub', () => {
    const visibility = computeVisibility(layout, levelOfDetail(layout, 1), false);
    expect(visibility.areFilesShown).toBe(false);
    expect(Array.from(visibility.visibleFiles)).toEqual([]);
    // Every hub is open, so each file stands in as the hub it sits in.
    expect(Array.from(visibility.fileStandIn)).toEqual([0, 1, 1, 3, 2, 2, 4]);
  });
});
