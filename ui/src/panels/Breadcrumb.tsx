// The selected file's folders, from the repository down: each a button that frames that folder
// whole and open, keeping the file selected. The way back out from a file to any folder above it,
// and the keyboard's way into a folder the map draws collapsed.
import type { ReactElement } from 'react';
import { Button } from '../components/ui/button';
import { useMapStore } from '../state/store';

/** The repository's crumb, and what it is called. */
const ROOT_NAME = 'Repository';

/** Every folder above `path`, from the repository ('') down to the file's own folder. */
export function foldersOf(path: string): string[] {
  const segments = path.split('/').slice(0, -1);
  return ['', ...segments.map((_, index) => segments.slice(0, index + 1).join('/'))];
}

/** The breadcrumb of the folders of file `path`. */
export function Breadcrumb({ path }: { path: string }): ReactElement {
  const showFolder = useMapStore((state) => state.showFolder);
  const folders = foldersOf(path);
  return (
    <nav className="breadcrumb" aria-label="Folders of the selected file">
      <ol>
        {folders.map((folder) => {
          const name = folder === '' ? ROOT_NAME : folder.slice(folder.lastIndexOf('/') + 1);
          const label = folder === '' ? 'Show the whole repository' : `Show the folder ${folder}/`;
          return (
            <li key={folder}>
              <Button
                variant="ghost"
                size="compact"
                className="crumb font-normal text-muted-foreground hover:text-foreground"
                aria-label={label}
                title={label}
                onClick={() => {
                  showFolder(folder);
                }}
              >
                {name}
              </Button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
