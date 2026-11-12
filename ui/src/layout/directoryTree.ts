// Builds the directory tree of a path list as a d3 hierarchy with children in name order.
// Directories are not sent by the daemon; they are derived from the paths here.
import { hierarchy, type HierarchyNode } from 'd3-hierarchy';
import { compareBytes } from '../text/byteOrder';

/** One directory: its own files (indexes into the path list) and its sub-directories. */
export interface DirectoryNode {
  /** The last path segment; empty for the root. */
  readonly name: string;
  /** The directory's path from the root, `/`-separated; empty for the root. */
  readonly path: string;
  /** Indexes of the files directly in this directory, in path order. */
  readonly files: number[];
  readonly dirs: DirectoryNode[];
}

function parentPathOf(path: string): string {
  const slash = path.lastIndexOf('/');
  return slash < 0 ? '' : path.slice(0, slash);
}

function nameOf(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

/** Finds the directory at `path`, creating it and any missing ancestors, without recursion. */
function ensureDirectory(path: string, known: Map<string, DirectoryNode>): DirectoryNode {
  const missing: string[] = [];
  let cursor = path;
  let found = known.get(cursor);
  while (found === undefined) {
    missing.push(cursor);
    cursor = parentPathOf(cursor);
    found = known.get(cursor);
  }
  let parent = found;
  for (let i = missing.length - 1; i >= 0; i -= 1) {
    const dirPath = missing[i] ?? '';
    const node: DirectoryNode = { name: nameOf(dirPath), path: dirPath, files: [], dirs: [] };
    parent.dirs.push(node);
    known.set(dirPath, node);
    parent = node;
  }
  return parent;
}

/**
 * Builds the tree for `paths`, which must be in byte order so each directory's files come out in
 * path order. Sub-directories are sorted by name at every level.
 */
export function buildDirectoryTree(paths: readonly string[]): HierarchyNode<DirectoryNode> {
  const root: DirectoryNode = { name: '', path: '', files: [], dirs: [] };
  const known = new Map<string, DirectoryNode>([['', root]]);
  for (const [index, path] of paths.entries()) {
    ensureDirectory(parentPathOf(path), known).files.push(index);
  }
  return hierarchy(root, (node) => node.dirs).sort((a, b) =>
    compareBytes(a.data.name, b.data.name),
  );
}
