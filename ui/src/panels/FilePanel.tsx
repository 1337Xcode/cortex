// The side panel for the selected file. Opens on the right (a bottom sheet on narrow screens);
// takes focus when the selection came from the keyboard; Escape or Close clears the selection and
// returns focus to where it was before the panel opened.
import { useEffect, useLayoutEffect, useRef, type ReactElement } from 'react';
import { fetchFileDetail } from '../api/file';
import type { Scene } from '../map/scene';
import { useMapStore } from '../state/store';
import { baseName } from '../text/format';
import {
  FactsSection,
  HistorySection,
  LinksSection,
  NumbersSection,
  ToldSection,
} from './fileSections';

/** Clears the selection and returns focus on Escape, unless something nearer used the key. */
function useEscapeToClose(close: () => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault();
      close();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
    };
  }, [close]);
}

/**
 * Frames the folder of file `id`, open: the keyboard's way into a folder the map draws collapsed
 * (search for any file in it, then this button).
 */
function FolderButton({ scene, id }: { scene: Scene; id: number }): ReactElement {
  const showFolderOf = useMapStore((state) => state.showFolderOf);
  const folder = scene.layout.hubPath[scene.layout.fileHub[id] ?? 0] ?? '';
  return (
    <button
      type="button"
      className="pill panel-folder"
      onClick={() => {
        showFolderOf(id);
      }}
    >
      {folder === '' ? 'Show the whole repository' : `Show the folder ${folder}/`}
    </button>
  );
}

/** The panel for file `id` of `scene`. */
export function FilePanel({
  scene,
  id,
  onClosed,
}: {
  scene: Scene;
  id: number;
  onClosed: (returnTo: Element | null) => void;
}): ReactElement {
  const detail = useMapStore((state) => state.detail);
  const flyTo = useMapStore((state) => state.flyTo);
  const selectFile = useMapStore((state) => state.selectFile);
  const loadDetail = useMapStore((state) => state.loadDetail);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const returnTo = useRef<Element | null>(null);
  const file = scene.snapshot.files[id];

  useLayoutEffect(() => {
    returnTo.current = document.activeElement;
  }, []);
  useEffect(() => {
    void loadDetail(fetchFileDetail);
  }, [id, loadDetail]);
  useEffect(() => {
    if (flyTo?.file === id && flyTo.target === 'file') headingRef.current?.focus();
  }, [flyTo, id]);

  const close = (): void => {
    selectFile(null);
    onClosed(returnTo.current);
  };
  useEscapeToClose(close);

  return (
    <aside className="panel" aria-labelledby="panel-title">
      <header className="panel-header">
        <div className="panel-title-block">
          <h2 id="panel-title" ref={headingRef} tabIndex={-1}>
            {baseName(file?.path ?? '')}
          </h2>
          <p className="panel-path">{file?.path}</p>
          <FolderButton scene={scene} id={id} />
        </div>
        <button
          type="button"
          className="pill panel-close"
          aria-label="Close the file panel"
          aria-keyshortcuts="Escape"
          onClick={close}
        >
          Close
        </button>
      </header>
      {file === undefined ? null : <NumbersSection file={file} />}
      <LinksSection scene={scene} id={id} />
      <FactsSection detail={detail} id={id} />
      <HistorySection detail={detail} id={id} />
      <ToldSection detail={detail} id={id} />
    </aside>
  );
}
