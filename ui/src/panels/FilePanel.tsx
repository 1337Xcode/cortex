// The panel of the selected file. Opens on the right (a bottom sheet on narrow screens); takes
// focus when the selection came from the keyboard; Escape or Close clears the selection and returns
// focus to where it was before the panel opened. Its header holds the breadcrumb of the file's
// folders, its name (after its file's icon) and path, and the link that opens it in an editor; its numbers come next, then sections that collapse to their title
// and count: the linked files and the facts open (what usually changes with this file, and which
// rules apply to it, the questions asked most), the history and what Cortex told agents closed
// until asked.
import { SquareArrowOutUpRight, X } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, type ReactElement } from 'react';
import { fetchFileDetail } from '../api/file';
import { ClippedText } from '../components/Clipped';
import { Button } from '../components/ui/button';
import { fileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import type { Scene } from '../map/scene';
import { useMapStore } from '../state/store';
import { baseName } from '../text/format';
import { Breadcrumb } from './Breadcrumb';
import { editorHref } from './editorLink';
import {
  detailFor,
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

/** "Open in editor", when the file's detail carries a link that may be followed. */
function EditorLink({ href }: { href: string | null }): ReactElement | null {
  if (href === null) return null;
  return (
    <Button asChild variant="link" size="inline" className="editor-link">
      <a href={href} rel="noreferrer">
        <SquareArrowOutUpRight aria-hidden="true" />
        Open in editor
      </a>
    </Button>
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
  const path = file?.path ?? '';
  const own = detailFor(detail, id);
  const href = own.status === 'ready' ? editorHref(own.detail.editor_url) : null;
  const icon = fileIcon(path);

  useLayoutEffect(() => {
    returnTo.current = document.activeElement;
  }, []);
  useEffect(() => {
    void loadDetail(fetchFileDetail);
  }, [id, loadDetail]);
  useEffect(() => {
    if (flyTo?.target === 'file' && flyTo.file === id) headingRef.current?.focus();
  }, [flyTo, id]);

  const close = (): void => {
    selectFile(null);
    onClosed(returnTo.current);
  };
  useEscapeToClose(close);

  return (
    <aside className="panel file-panel" aria-labelledby="panel-title">
      <header className="panel-header">
        <div className="panel-header-row">
          <Breadcrumb path={path} />
          <Button
            variant="ghost"
            size="icon-compact"
            className="panel-close text-muted-foreground hover:text-foreground"
            aria-label="Close the file panel"
            aria-keyshortcuts="Escape"
            title="Close (Escape)"
            onClick={close}
          >
            <X aria-hidden="true" />
          </Button>
        </div>
        <div className="panel-title-row">
          {icon === null ? null : <IconSlot icon={icon} />}
          <h2 id="panel-title" className="panel-title" ref={headingRef} tabIndex={-1}>
            <ClippedText text={baseName(path)} kind="words" />
          </h2>
        </div>
        <ClippedText text={path} kind="path" className="panel-path" />
        <EditorLink href={href} />
      </header>
      <div className="panel-body" data-scroll-area="true">
        {file === undefined ? null : <NumbersSection file={file} />}
        <LinksSection scene={scene} id={id} />
        <FactsSection detail={detail} id={id} />
        <HistorySection detail={detail} id={id} />
        <ToldSection detail={detail} id={id} />
      </div>
    </aside>
  );
}
