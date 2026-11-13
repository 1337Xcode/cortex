// The overview panel on the left, in the order of how often a person asks its questions: the agent
// sessions running now and what agents did lately, the repository at a glance (with when its
// numbers were read), the receipts, the files that change together, the busiest files and what
// needs attention, then the warnings Cortex gave and the daemon's health. Each list shows five rows
// with "Show N more", every file row a button that flies to its file, and each section collapses to
// its title and count (the full count, or "top 25 of 2,194" when the list is cut). The live
// sessions and the activity open while a session runs; the receipts, the warnings and the health,
// asked for when opened, start closed; the other four are open at first, since each answers a
// question asked on first sight. On a narrow screen it is a bottom sheet opened from the controls,
// with a Close button.
import { X } from 'lucide-react';
import { useMemo, type ReactElement, type RefObject } from 'react';
import type { GraphSnapshot } from '../api/graph';
import type { Scene } from '../map/scene';
import { Button } from '../components/ui/button';
import { fileIcon, sharedFileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import { refreshesByItself } from '../state/liveRefresh';
import { PAGE_REFRESHERS } from '../state/pageRefreshers';
import { useMapStore } from '../state/store';
import { countOf, formatCount, formatUtcTime } from '../text/format';
import { distinctNames } from '../text/names';
import {
  KEPT_LINKS_NOTE,
  LINK_KIND_NAMES,
  NUMBER_NAMES,
  PAIR_ORDER_SENTENCE,
  linkMeasure,
  linkSentence,
} from '../text/vocabulary';
import { ActivitySection } from './ActivitySection';
import { FileRow } from './FileRow';
import { HealthSection } from './HealthSection';
import { LiveSessions } from './LiveSessions';
import { ReceiptsSection } from './ReceiptsSection';
import { busiestFiles, coupledPairs, filesNeedingAttention, listCount } from './overviewLists';
import { RowList } from './RowList';
import { Section } from './Section';
import { SentenceView, summaryText } from './SummaryStrip';
import { WarningsSection } from './WarningsSection';

/**
 * When the map's numbers were read, and, for a map too large to refresh by itself while agents
 * work, the button that reads them again.
 */
function ReadAt(): ReactElement | null {
  const load = useMapStore((state) => state.load);
  const refresh = useMapStore((state) => state.refresh);
  if (load.status !== 'ready') return null;
  const isAuto = refreshesByItself({ load });
  return (
    <>
      <p className="section-note" data-testid="overview-read-at">
        {`Numbers read at ${formatUtcTime(load.receivedMs)} UTC by this computer's clock${isAuto ? ', and read again while agents work.' : '.'}`}
      </p>
      {isAuto ? null : (
        <Button
          variant="ghost"
          size="compact"
          className="show-more -ml-sm text-muted-foreground hover:text-foreground"
          onClick={() => {
            void refresh(PAGE_REFRESHERS);
          }}
        >
          Read again
        </Button>
      )}
    </>
  );
}

/** Every summary sentence in full, each named for the checks by its key. */
function AtAGlance({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const summary = useMapStore((state) => state.summary);
  const text = useMemo(() => summaryText(summary, snapshot), [summary, snapshot]);
  return (
    <Section title="At a glance" count={null} isOpenAtFirst testId="overview-glance">
      {text === null ? (
        <p className="panel-empty">Loading the summary.</p>
      ) : !text.ok ? (
        <p className="panel-empty" role="alert">
          The summary could not be loaded ({text.error.code}): {text.error.message}. Reload the page
          to try again.
        </p>
      ) : (
        <ul className="glance-list">
          {text.sentences.map((sentence) => (
            <li key={sentence.key} data-testid={`summary-${sentence.key}`}>
              <SentenceView parts={sentence.parts} />
            </li>
          ))}
        </ul>
      )}
      <ReadAt />
    </Section>
  );
}

/** The strongest pairs of files changed together, ranked as the daemon ranks them. */
function ChangedTogether({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const top = useMemo(() => coupledPairs(snapshot), [snapshot]);
  const pairs = top.rows;
  return (
    <Section
      title={LINK_KIND_NAMES.co_change}
      count={listCount(top)}
      isOpenAtFirst
      testId="overview-pairs"
    >
      {pairs.length === 0 ? (
        <p className="panel-empty">
          No files have changed together yet. Cortex learns this from the repository's commits.
        </p>
      ) : (
        <p className="section-note">
          {PAIR_ORDER_SENTENCE} {KEPT_LINKS_NOTE}
        </p>
      )}
      {pairs.length === 0 ? null : (
        <RowList
          items={pairs}
          keyOf={(pair) => `${pair.link.from}:${pair.link.to}`}
          label="Files changed together, strongest first"
          renderRow={(pair) => {
            const [a, b] = distinctNames([pair.from.path, pair.to.path]);
            return (
              <FileRow
                file={pair.from.id}
                name={`${a ?? ''} and ${b ?? ''}`}
                nameKind="words"
                measure={linkMeasure(pair.link)}
                sentence={linkSentence(pair.link, pair.from, pair.to)}
                lead={<IconSlot icon={sharedFileIcon(pair.from.path, pair.to.path)} />}
              />
            );
          }}
        />
      )}
    </Section>
  );
}

/** The files agents worked on most, or the most changed files before any agent did. */
function Busiest({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const busiest = useMemo(() => busiestFiles(snapshot), [snapshot]);
  const isTokens = busiest.measure === 'tokens';
  const files = busiest.files.rows;
  const names = distinctNames(files.map((file) => file.path));
  return (
    <Section
      title="Busiest files"
      count={listCount(busiest.files)}
      isOpenAtFirst
      testId="overview-busiest"
    >
      {files.length === 0 ? (
        <p className="panel-empty">No file has been changed or worked on yet.</p>
      ) : (
        <p className="section-note">
          {isTokens
            ? `By ${NUMBER_NAMES.tokens.toLowerCase()}.`
            : 'By commits: agents have not worked on any file here yet.'}
        </p>
      )}
      {files.length === 0 ? null : (
        <RowList
          items={files.map((file, index) => ({ file, name: names[index] ?? file.path }))}
          keyOf={({ file }) => file.id}
          label="Busiest files"
          renderRow={({ file, name }) => {
            const measure = isTokens
              ? countOf(file.tokens, 'token')
              : countOf(file.commits, 'commit');
            return (
              <FileRow
                file={file.id}
                name={name}
                measure={measure}
                lead={<IconSlot icon={fileIcon(file.path)} />}
                sentence={`${file.path}: ${isTokens ? `${formatCount(file.tokens)} tokens attributed` : `changed in ${countOf(file.commits, 'commit')}`}.`}
              />
            );
          }}
        />
      )}
    </Section>
  );
}

/** The files with a suspect fact. */
function NeedsAttention({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const top = useMemo(() => filesNeedingAttention(snapshot), [snapshot]);
  const files = top.rows;
  const names = distinctNames(files.map((file) => file.path));
  return (
    <Section
      title="Needs attention"
      count={listCount(top)}
      isOpenAtFirst
      testId="overview-attention"
    >
      {files.length === 0 ? (
        <p className="panel-empty">Nothing needs attention: no fact is suspect.</p>
      ) : (
        <RowList
          items={files.map((file, index) => ({ file, name: names[index] ?? file.path }))}
          keyOf={({ file }) => file.id}
          label="Files with a suspect fact"
          renderRow={({ file, name }) => (
            <FileRow
              file={file.id}
              name={name}
              measure={countOf(file.suspect, 'suspect fact')}
              lead={<IconSlot icon={fileIcon(file.path)} />}
              sentence={`${file.path} has ${countOf(file.suspect, 'suspect fact')}: the code it describes may have changed.`}
            />
          )}
        />
      )}
    </Section>
  );
}

/** The overview panel; `isSheet` makes it the closable bottom sheet of a narrow screen. */
export function OverviewPanel({
  scene,
  isSheet,
  panelRef,
}: {
  scene: Scene;
  isSheet: boolean;
  panelRef: RefObject<HTMLElement | null>;
}): ReactElement {
  const setOverviewOpen = useMapStore((state) => state.setOverviewOpen);
  const { snapshot } = scene;
  return (
    <aside
      className="panel overview-panel"
      aria-label="Overview"
      ref={panelRef}
      data-sheet={isSheet}
    >
      {isSheet ? (
        <header className="panel-header-row overview-header">
          <h2 className="overview-title">Overview</h2>
          <Button
            variant="ghost"
            size="icon-compact"
            className="text-muted-foreground hover:text-foreground"
            aria-label="Close the overview"
            title="Close the overview"
            onClick={() => {
              setOverviewOpen(false);
            }}
          >
            <X aria-hidden="true" />
          </Button>
        </header>
      ) : (
        // On a wide screen the panel's sections name themselves; its name is for screen readers.
        <h2 className="visually-hidden">Overview</h2>
      )}
      <div className="panel-body" data-scroll-area="true">
        <LiveSessions scene={scene} />
        <ActivitySection scene={scene} />
        <AtAGlance snapshot={snapshot} />
        <ReceiptsSection />
        <ChangedTogether snapshot={snapshot} />
        <Busiest snapshot={snapshot} />
        <NeedsAttention snapshot={snapshot} />
        <WarningsSection scene={scene} />
        <HealthSection />
      </div>
    </aside>
  );
}
