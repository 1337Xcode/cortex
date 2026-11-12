// The file panel's numbers and sections, each with its own empty state: the file's counts, its
// strongest links by kind, the facts anchored to it (suspect ones marked), the stored history of
// its commits in pages of ten, and what Cortex told agents about it with whether they then used it.
// Every section collapses to its title and count.
import { useId, useState, type ReactElement, type ReactNode } from 'react';
import type { FactStatus, FileDetail, FileHistoryEntry, ToldKind } from '../api/file';
import { LINK_KINDS, type GraphFile, type LinkKind } from '../api/graph';
import { visibleText } from '../api/guard';
import { ClippedText } from '../components/Clipped';
import { Button } from '../components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../components/ui/tooltip';
import { fileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import type { Scene } from '../map/scene';
import { useMapStore, type DetailState } from '../state/store';
import { countOf, formatCount, formatDate } from '../text/format';
import { distinctNames } from '../text/names';
import {
  LINK_KIND_NAMES,
  NUMBER_NAMES,
  TOKENS_ATTRIBUTED_MEANING,
  commitsMeaning,
  linkSentence,
} from '../text/vocabulary';
import { LIST } from '../theme/tokens';
import { strongestLinks, type PanelLink } from './fileLinks';
import { FileRow } from './FileRow';
import { RowList } from './RowList';
import { Section } from './Section';

const STATUS_NAMES: Readonly<Record<FactStatus, string>> = {
  active: 'Active',
  candidate: 'Candidate',
  suspect: 'Suspect',
};

const TOLD_NAMES: Readonly<Record<ToldKind, string>> = {
  work_order: 'Work order',
  warning: 'Warning',
};

/** One of the file's numbers: its name, its value, and what it counts when that needs saying. */
interface NumberRow {
  readonly name: string;
  readonly value: number;
  readonly meaning: string | null;
}

/**
 * A number that needs explaining: it takes keyboard focus and says what it counts in a tooltip on
 * hover and focus; its `dd` is described by the same sentence for screen readers.
 */
function ExplainedNumber({ value, meaning }: { value: number; meaning: string }): ReactElement {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="number-explained" tabIndex={0}>
          {formatCount(value)}
        </span>
      </TooltipTrigger>
      <TooltipContent>{meaning}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The file's counts as a definition list; each name says exactly what it counts, and the commits
 * and tokens, whose meaning is not plain from the name, explain themselves on hover and focus.
 */
export function NumbersSection({ file }: { file: GraphFile }): ReactElement {
  const idPrefix = useId();
  const summary = useMapStore((state) => state.summary);
  const limits = summary.status === 'ready' ? summary.summary.history : null;
  const rows: NumberRow[] = [
    { name: NUMBER_NAMES.lines, value: file.lines, meaning: null },
    { name: NUMBER_NAMES.reads, value: file.reads, meaning: null },
    {
      name: NUMBER_NAMES.commits,
      value: file.commits,
      meaning: commitsMeaning(file.commits, limits),
    },
    { name: NUMBER_NAMES.tokens, value: file.tokens, meaning: TOKENS_ATTRIBUTED_MEANING },
  ];
  return (
    <>
      <dl className="panel-numbers">
        {rows.map((row, index) => (
          <div key={row.name}>
            <dt>{row.name}</dt>
            <dd aria-describedby={row.meaning === null ? undefined : `${idPrefix}-${index}`}>
              {row.meaning === null ? (
                formatCount(row.value)
              ) : (
                <ExplainedNumber value={row.value} meaning={row.meaning} />
              )}
            </dd>
          </div>
        ))}
      </dl>
      {rows.map((row, index) =>
        row.meaning === null ? null : (
          <p key={row.name} id={`${idPrefix}-${index}`} className="visually-hidden">
            {row.meaning}
          </p>
        ),
      )}
    </>
  );
}

/** One linked file as a row that flies to it, with what the link counts. */
function linkRow(
  scene: Scene,
  id: number,
  kind: LinkKind,
  link: PanelLink,
  name: string,
): ReactNode {
  const self = scene.snapshot.files[id];
  const other = scene.snapshot.files[link.other];
  if (self === undefined || other === undefined) return null;
  const measure =
    kind === 'co_change' ? countOf(link.commits, 'commit') : `strength ${link.weight.toFixed(2)}`;
  const graphLink = { from: id, to: link.other, kind, weight: link.weight, commits: link.commits };
  return (
    <FileRow
      file={link.other}
      name={name}
      nameKind="words"
      measure={measure}
      sentence={linkSentence(graphLink, self, other)}
      lead={<IconSlot icon={fileIcon(other.path)} />}
    />
  );
}

/** The file's strongest links of each kind; open at first. */
export function LinksSection({ scene, id }: { scene: Scene; id: number }): ReactElement {
  const grouped = strongestLinks(scene, id, LIST.most);
  // Names are told apart across every list the panel shows at once.
  const shown = LINK_KINDS.flatMap((kind) => grouped[kind].map((link) => link.path));
  const distinct = distinctNames(shown);
  const names = new Map(shown.map((path, i) => [path, distinct[i] ?? path]));
  const total = LINK_KINDS.reduce((sum, kind) => sum + grouped[kind].length, 0);
  return (
    <Section title="Linked files" count={total} isOpenAtFirst testId="panel-links">
      {LINK_KINDS.map((kind) => (
        <div key={kind} className="panel-kind" data-testid={`panel-links-${kind}`}>
          <h4>{LINK_KIND_NAMES[kind]}</h4>
          {grouped[kind].length === 0 ? (
            <p className="panel-empty">None learned yet.</p>
          ) : (
            <RowList
              items={grouped[kind]}
              keyOf={(link) => link.other}
              label={LINK_KIND_NAMES[kind]}
              renderRow={(link) =>
                linkRow(scene, id, kind, link, names.get(link.path) ?? link.path)
              }
            />
          )}
        </div>
      ))}
    </Section>
  );
}

/**
 * The detail state as the panel for file `id` may show it: a detail or error for another file
 * (a response that outran a new selection) reads as still loading, never as this file's.
 */
export function detailFor(detail: DetailState, id: number): DetailState {
  const isOther =
    (detail.status === 'ready' && detail.detail.id !== id) ||
    ((detail.status === 'error' || detail.status === 'loading') && detail.id !== id);
  return isOther ? { status: 'loading', id } : detail;
}

/** One section fed by the file's detail, with its title, count, test id and empty text. */
function DetailSection(props: {
  title: string;
  testId: string;
  detail: DetailState;
  id: number;
  isOpenAtFirst: boolean;
  countOf: (detail: FileDetail) => number;
  empty: string;
  children: (detail: FileDetail) => ReactElement;
}): ReactElement {
  const detail = detailFor(props.detail, props.id);
  const count = detail.status === 'ready' ? props.countOf(detail.detail) : null;
  return (
    <Section
      title={props.title}
      count={count}
      isOpenAtFirst={props.isOpenAtFirst}
      testId={props.testId}
    >
      {detail.status === 'error' ? (
        <p className="panel-empty" role="alert">
          Could not load this ({detail.error.code}): {detail.error.message}. Select the file again
          to retry.
        </p>
      ) : detail.status !== 'ready' ? (
        <p className="panel-empty">Loading.</p>
      ) : count === 0 ? (
        <p className="panel-empty">{props.empty}</p>
      ) : (
        props.children(detail.detail)
      )}
    </Section>
  );
}

/** The facts anchored to the file; suspect ones are marked in words and colour. Open at first. */
export function FactsSection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title="Facts"
      testId="panel-facts"
      detail={detail}
      id={id}
      isOpenAtFirst
      countOf={(d) => d.facts.length}
      empty="No facts are recorded for this file."
    >
      {(d) => (
        <ul className="fact-list">
          {d.facts.map((fact) => (
            <li key={fact.id} data-status={fact.status}>
              <span className="fact-status">{STATUS_NAMES[fact.status]}</span>
              <span className="fact-kind">{visibleText(fact.kind)}</span>
              <p className="fact-text">{visibleText(fact.text)}</p>
            </li>
          ))}
        </ul>
      )}
    </DetailSection>
  );
}

/** "11 to 20 of 34": the rows a history page shows. */
export function pageRange(page: number, total: number): string {
  const first = page * LIST.historyPage + 1;
  const last = Math.min(total, (page + 1) * LIST.historyPage);
  return `${formatCount(first)} to ${formatCount(last)} of ${formatCount(total)}`;
}

/** The history in pages of ten, newest first, with buttons to the newer and older pages. */
function HistoryPages({ history }: { history: readonly FileHistoryEntry[] }): ReactElement {
  const [page, setPage] = useState(0);
  const pages = Math.ceil(history.length / LIST.historyPage);
  const rows = history.slice(page * LIST.historyPage, (page + 1) * LIST.historyPage);
  return (
    <>
      <ul className="history-list">
        {rows.map((entry, index) => (
          <li key={page * LIST.historyPage + index}>
            <ClippedText
              text={visibleText(entry.subject)}
              kind="words"
              className="history-subject"
            />
            <span className="history-meta">
              {formatDate(entry.date_s * 1000)}
              {entry.refs === '' ? '' : ` ${visibleText(entry.refs)}`}
            </span>
          </li>
        ))}
      </ul>
      {pages > 1 ? (
        <nav className="pager" aria-label="History pages">
          <Button
            size="compact"
            disabled={page === 0}
            onClick={() => {
              setPage(page - 1);
            }}
          >
            Newer
          </Button>
          <span className="pager-range" aria-live="polite">
            {pageRange(page, history.length)}
          </span>
          <Button
            size="compact"
            disabled={page >= pages - 1}
            onClick={() => {
              setPage(page + 1);
            }}
          >
            Older
          </Button>
        </nav>
      ) : null}
    </>
  );
}

/** The stored commit subjects that touched the file, newest first. Closed at first. */
export function HistorySection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title="History"
      testId="panel-history"
      detail={detail}
      id={id}
      isOpenAtFirst={false}
      countOf={(d) => d.history.length}
      empty="No commit history read for this file yet; Cortex fills it in as it reads the repository's history."
    >
      {(d) => <HistoryPages history={d.history} />}
    </DetailSection>
  );
}

function usedText(used: boolean | null): string {
  if (used === null) return 'not known yet';
  return used ? 'then read or edited' : 'not used';
}

/** The times Cortex named the file to an agent, newest first, and whether the agent used it. */
export function ToldSection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title="What Cortex told agents"
      testId="panel-told"
      detail={detail}
      id={id}
      isOpenAtFirst={false}
      countOf={(d) => d.told.length}
      empty="Cortex has not named this file to an agent yet."
    >
      {(d) => (
        <RowList
          items={d.told.map((entry, index) => ({ entry, index }))}
          keyOf={({ index }) => index}
          label="Times Cortex named this file"
          renderRow={({ entry }) => (
            <p className="told-row" data-used={String(entry.used)}>
              <span>{TOLD_NAMES[entry.kind]}</span>
              <span className="told-meta">
                {formatDate(entry.at_ms)}, {usedText(entry.used)}
              </span>
            </p>
          )}
        />
      )}
    </DetailSection>
  );
}
