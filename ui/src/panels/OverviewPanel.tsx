// The overview panel on the left, in the order of how often a person asks its questions: what the
// repository is (the summary sentences the strip on top does not show in full), the files that
// change together, the busiest files, the agent sessions running now and what agents did lately,
// what needs attention, the warnings Cortex gave, the receipts and the daemon's health; then when
// its numbers were read. Each list shows five rows with "Show N more", every file row a button
// that flies to its file, and each section collapses to its title and count (the full count, or
// "top 25 of 2,194" when the list is cut). The lists open when they have rows; the live sessions
// and the activity open while a session runs; the warnings, the receipts and the health, asked
// for when opened, start closed, and the activity, the warnings and the receipts open on their
// letter from anywhere. On a narrow screen it is a bottom sheet opened from the controls, with a
// Close button.
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
import { namesOfFiles } from '../text/pathNames';
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
import { Footnote, Section } from './Section';
import { SentenceView, summaryText } from './SummaryStrip';
import { WarningsSection } from './WarningsSection';

/**
 * When the map's numbers were read, and, for a map too large to refresh by itself while agents
 * work, the button that reads them again: the panel's last line.
 */
function ReadAt(): ReactElement | null {
  const load = useMapStore((state) => state.load);
  const refresh = useMapStore((state) => state.refresh);
  if (load.status !== 'ready') return null;
  const isAuto = refreshesByItself({ load });
  return (
    <footer className="overview-footer">
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
    </footer>
  );
}

/**
 * The summary sentences the strip does not show in full (`inStrip` holds the keys it does, joined
 * by spaces), each named for the checks by its key; nothing when the strip shows them all.
 */
function AtAGlance({
  snapshot,
  inStrip,
}: {
  snapshot: GraphSnapshot;
  inStrip: string;
}): ReactElement | null {
  const summary = useMapStore((state) => state.summary);
  const text = useMemo(() => summaryText(summary, snapshot), [summary, snapshot]);
  const names = namesOfFiles(snapshot.files);
  if (text?.ok === true) {
    const shown = new Set(inStrip.split(' '));
    const rest = text.sentences.filter((sentence) => !shown.has(sentence.key));
    if (rest.length === 0) return null;
    return (
      <Section title="At a glance" count={null} isOpenAtFirst testId="overview-glance">
        <ul className="glance-list">
          {rest.map((sentence) => (
            <li key={sentence.key} data-testid={`summary-${sentence.key}`}>
              <SentenceView parts={sentence.parts} names={names} />
            </li>
          ))}
        </ul>
      </Section>
    );
  }
  return (
    <Section title="At a glance" count={null} isOpenAtFirst testId="overview-glance">
      {text === null ? (
        <p className="panel-empty">Loading the summary.</p>
      ) : (
        <p className="panel-empty" role="alert">
          The summary could not be loaded ({text.error.code}): {text.error.message}. Reload the page
          to try again.
        </p>
      )}
    </Section>
  );
}

/** The strongest pairs of files changed together, ranked as the daemon ranks them. */
function ChangedTogether({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const top = useMemo(() => coupledPairs(snapshot), [snapshot]);
  const pairs = top.rows;
  const names = namesOfFiles(snapshot.files);
  const hasRows = pairs.length > 0;
  return (
    <Section
      title={LINK_KIND_NAMES.co_change}
      count={listCount(top)}
      isOpenAtFirst={hasRows}
      openWhile={hasRows}
      testId="overview-pairs"
    >
      {hasRows ? (
        <>
          <RowList
            items={pairs}
            keyOf={(pair) => `${pair.link.from}:${pair.link.to}`}
            label="Files changed together, strongest first"
            renderRow={(pair) => (
              <FileRow
                file={pair.from.id}
                name={{ first: names.nameOf(pair.from.path), second: names.nameOf(pair.to.path) }}
                measure={linkMeasure(pair.link)}
                sentence={linkSentence(pair.link, pair.from, pair.to)}
                lead={<IconSlot icon={sharedFileIcon(pair.from.path, pair.to.path)} />}
              />
            )}
          />
          <Footnote text={`${PAIR_ORDER_SENTENCE} ${KEPT_LINKS_NOTE}`} />
        </>
      ) : (
        <p className="panel-empty">
          No files have changed together yet. Cortex learns this from the repository&apos;s commits.
        </p>
      )}
    </Section>
  );
}

/** The files agents worked on most, or the most changed files before any agent did. */
function Busiest({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const busiest = useMemo(() => busiestFiles(snapshot), [snapshot]);
  const isTokens = busiest.measure === 'tokens';
  const files = busiest.files.rows;
  const names = namesOfFiles(snapshot.files);
  const hasRows = files.length > 0;
  return (
    <Section
      title="Busiest files"
      count={listCount(busiest.files)}
      isOpenAtFirst={hasRows}
      openWhile={hasRows}
      testId="overview-busiest"
    >
      {hasRows ? (
        <p className="section-note">
          {isTokens
            ? `By ${NUMBER_NAMES.tokens.toLowerCase()}.`
            : 'By commits: agents have not worked on any file here yet.'}
        </p>
      ) : (
        <p className="panel-empty">No file has been changed or worked on yet.</p>
      )}
      {hasRows ? (
        <RowList
          items={files}
          keyOf={(file) => file.id}
          label="Busiest files"
          renderRow={(file) => (
            <FileRow
              file={file.id}
              name={names.nameOf(file.path)}
              measure={isTokens ? countOf(file.tokens, 'token') : countOf(file.commits, 'commit')}
              lead={<IconSlot icon={fileIcon(file.path)} />}
              sentence={`${file.path}: ${isTokens ? `${formatCount(file.tokens)} tokens attributed` : `changed in ${countOf(file.commits, 'commit')}`}.`}
            />
          )}
        />
      ) : null}
    </Section>
  );
}

/** The files with a suspect fact. */
function NeedsAttention({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const top = useMemo(() => filesNeedingAttention(snapshot), [snapshot]);
  const files = top.rows;
  const names = namesOfFiles(snapshot.files);
  const hasRows = files.length > 0;
  return (
    <Section
      title="Needs attention"
      count={listCount(top)}
      isOpenAtFirst={hasRows}
      openWhile={hasRows}
      testId="overview-attention"
    >
      {hasRows ? (
        <RowList
          items={files}
          keyOf={(file) => file.id}
          label="Files with a suspect fact"
          renderRow={(file) => (
            <FileRow
              file={file.id}
              name={names.nameOf(file.path)}
              measure={countOf(file.suspect, 'suspect fact')}
              lead={<IconSlot icon={fileIcon(file.path)} />}
              sentence={`${file.path} has ${countOf(file.suspect, 'suspect fact')}: the code it describes may have changed.`}
            />
          )}
        />
      ) : (
        <p className="panel-empty">Nothing needs attention: no fact is suspect.</p>
      )}
    </Section>
  );
}

/**
 * The overview panel; `isSheet` makes it the closable bottom sheet of a narrow screen, and
 * `inStrip` holds the keys of the summary sentences the strip shows in full, joined by spaces.
 */
export function OverviewPanel({
  scene,
  isSheet,
  inStrip,
  panelRef,
}: {
  scene: Scene;
  isSheet: boolean;
  inStrip: string;
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
        <AtAGlance snapshot={snapshot} inStrip={inStrip} />
        <ChangedTogether snapshot={snapshot} />
        <Busiest snapshot={snapshot} />
        <LiveSessions scene={scene} />
        <ActivitySection scene={scene} />
        <NeedsAttention snapshot={snapshot} />
        <WarningsSection scene={scene} />
        <ReceiptsSection />
        <HealthSection />
        <ReadAt />
      </div>
    </aside>
  );
}
