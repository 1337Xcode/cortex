// The overview panel on the left: the agent sessions running now, the repository at a glance, the
// files that change together, the busiest files and what needs attention, each a short list of
// five with "Show N more" and every row a button that flies to its file. Each section collapses to
// its title and count; the live sessions open while any runs, and the other four are open at
// first, since each answers a question asked on first sight. On a narrow screen it is a bottom
// sheet opened from the controls, with a Close button.
import { X } from 'lucide-react';
import { useMemo, type ReactElement, type RefObject } from 'react';
import type { GraphSnapshot } from '../api/graph';
import type { Scene } from '../map/scene';
import { Button } from '../components/ui/button';
import { useMapStore } from '../state/store';
import { countOf, formatCount } from '../text/format';
import { distinctNames } from '../text/names';
import { LINK_KIND_NAMES, NUMBER_NAMES, linkSentence } from '../text/vocabulary';
import { FileRow } from './FileRow';
import { LiveSessions } from './LiveSessions';
import { busiestFiles, coupledPairs, filesNeedingAttention } from './overviewLists';
import { RowList } from './RowList';
import { Section } from './Section';
import { SentenceView, summaryText } from './SummaryStrip';

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
    </Section>
  );
}

/** The pairs of files changed together most often. */
function ChangedTogether({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const pairs = useMemo(() => coupledPairs(snapshot), [snapshot]);
  return (
    <Section
      title={LINK_KIND_NAMES.co_change}
      count={pairs.length}
      isOpenAtFirst
      testId="overview-pairs"
    >
      {pairs.length === 0 ? (
        <p className="panel-empty">
          No files have changed together yet. Cortex learns this from the repository's commits.
        </p>
      ) : (
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
                measure={countOf(pair.link.commits, 'commit')}
                sentence={linkSentence(pair.link, pair.from, pair.to)}
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
  const names = distinctNames(busiest.files.map((file) => file.path));
  return (
    <Section
      title="Busiest files"
      count={busiest.files.length}
      isOpenAtFirst
      testId="overview-busiest"
    >
      {busiest.files.length === 0 ? (
        <p className="panel-empty">No file has been changed or worked on yet.</p>
      ) : (
        <p className="section-note">
          {isTokens
            ? `By ${NUMBER_NAMES.tokens.toLowerCase()}.`
            : 'By commits: agents have not worked on any file here yet.'}
        </p>
      )}
      {busiest.files.length === 0 ? null : (
        <RowList
          items={busiest.files.map((file, index) => ({ file, name: names[index] ?? file.path }))}
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
  const files = useMemo(() => filesNeedingAttention(snapshot), [snapshot]);
  const names = distinctNames(files.map((file) => file.path));
  return (
    <Section title="Needs attention" count={files.length} isOpenAtFirst testId="overview-attention">
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
        <AtAGlance snapshot={snapshot} />
        <ChangedTogether snapshot={snapshot} />
        <Busiest snapshot={snapshot} />
        <NeedsAttention snapshot={snapshot} />
      </div>
    </aside>
  );
}
