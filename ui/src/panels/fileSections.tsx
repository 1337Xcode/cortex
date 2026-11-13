// The file panel's numbers and sections, each with its own empty state: the file's counts, its
// strongest links by kind (each with the count its strength was made from, and how many the map
// keeps), the newest commits behind its current lines with how many there are, and what Cortex told
// agents about it with whether they then used it. Every section collapses to its title and count.
// The facts and the symbols have files of their own.
import { useId, type ReactElement, type ReactNode } from 'react';
import { MAX_TOLD, type FileDetail, type ToldKind } from '../api/file';
import { LINK_KINDS, type GraphFile, type LinkKind } from '../api/graph';
import { visibleText } from '../api/guard';
import { ClippedText } from '../components/Clipped';
import { Tooltip, TooltipContent, TooltipTrigger } from '../components/ui/tooltip';
import { fileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import type { Scene } from '../map/scene';
import { useMapStore, type DetailState } from '../state/store';
import { formatCount, formatDate } from '../text/format';
import { HISTORY_TITLE, historyNote } from '../text/historyWords';
import { distinctNames } from '../text/names';
import {
  LINK_KIND_NAMES,
  NOT_COUNTED,
  NUMBER_NAMES,
  KEPT_LINKS_NOTE,
  TOKENS_ATTRIBUTED_MEANING,
  commitsMeaning,
  keptLinksText,
  linkMeasure,
  linkSentence,
} from '../text/vocabulary';
import { LIST } from '../theme/tokens';
import { strongestLinks, type PanelLink } from './fileLinks';
import { FileRow } from './FileRow';
import { listCount } from './overviewLists';
import { RowList } from './RowList';
import { Section } from './Section';

const TOLD_NAMES: Readonly<Record<ToldKind, string>> = {
  work_order: 'Work order',
  warning: 'Warning',
};

/** One of the file's numbers: its name, its value as shown, and what it counts when that needs saying. */
interface NumberRow {
  readonly name: string;
  readonly value: string;
  readonly meaning: string | null;
}

/**
 * A number that needs explaining: it takes keyboard focus and says what it counts in a tooltip on
 * hover and focus; its `dd` is described by the same sentence for screen readers.
 */
function ExplainedNumber({ value, meaning }: { value: string; meaning: string }): ReactElement {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="number-explained" tabIndex={0}>
          {value}
        </span>
      </TooltipTrigger>
      <TooltipContent>{meaning}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The file's counts as a definition list; each name says exactly what it counts, and the commits
 * and tokens, whose meaning is not plain from the name, explain themselves on hover and focus. A
 * generated or vendored file's commits are not counted, and it says so instead of a 0.
 */
export function NumbersSection({ file }: { file: GraphFile }): ReactElement {
  const idPrefix = useId();
  const summary = useMapStore((state) => state.summary);
  const limits = summary.status === 'ready' ? summary.summary.history : null;
  const rows: NumberRow[] = [
    { name: NUMBER_NAMES.lines, value: formatCount(file.lines), meaning: null },
    { name: NUMBER_NAMES.reads, value: formatCount(file.reads), meaning: null },
    {
      name: NUMBER_NAMES.commits,
      value: file.commits_counted ? formatCount(file.commits) : NOT_COUNTED,
      meaning: commitsMeaning(file, limits),
    },
    {
      name: NUMBER_NAMES.tokens,
      value: formatCount(file.tokens),
      meaning: TOKENS_ATTRIBUTED_MEANING,
    },
  ];
  return (
    <>
      <dl className="panel-numbers">
        {rows.map((row, index) => (
          <div key={row.name}>
            <dt>{row.name}</dt>
            <dd aria-describedby={row.meaning === null ? undefined : `${idPrefix}-${index}`}>
              {row.meaning === null ? (
                row.value
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
  const graphLink = {
    from: id,
    to: link.other,
    kind,
    weight: link.weight,
    commits: link.commits,
    count: link.count,
  };
  return (
    <FileRow
      file={link.other}
      name={name}
      nameKind="words"
      measure={linkMeasure(graphLink)}
      sentence={linkSentence(graphLink, self, other)}
      lead={<IconSlot icon={fileIcon(other.path)} />}
    />
  );
}

/**
 * The file's strongest links of each kind, strongest first as the daemon ranks them; open at
 * first. The count says how many the map keeps, and how many are shown when that is fewer.
 */
export function LinksSection({ scene, id }: { scene: Scene; id: number }): ReactElement {
  const { shown: grouped, totals } = strongestLinks(scene, id, LIST.most);
  // Names are told apart across every list the panel shows at once.
  const shown = LINK_KINDS.flatMap((kind) => grouped[kind].map((link) => link.path));
  const distinct = distinctNames(shown);
  const names = new Map(shown.map((path, i) => [path, distinct[i] ?? path]));
  const count = listCount({
    rows: shown,
    total: LINK_KINDS.reduce((sum, kind) => sum + totals[kind], 0),
  });
  return (
    <Section title="Linked files" count={count} isOpenAtFirst testId="panel-links">
      {shown.length === 0 ? null : <p className="section-note">{KEPT_LINKS_NOTE}</p>}
      {LINK_KINDS.map((kind) => (
        <div key={kind} className="panel-kind" data-testid={`panel-links-${kind}`}>
          <h4>
            {LINK_KIND_NAMES[kind]}
            {grouped[kind].length < totals[kind]
              ? ` (${keptLinksText(grouped[kind].length, totals[kind])})`
              : ''}
          </h4>
          {grouped[kind].length === 0 ? (
            <p className="panel-empty">None learned yet.</p>
          ) : (
            <RowList
              items={grouped[kind]}
              keyOf={(link) => link.other}
              label={`${LINK_KIND_NAMES[kind]}, strongest first`}
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

/**
 * One section fed by the file's detail, with its title, count, test id and empty text, and a
 * footer shown in every state (the facts say there what came of the person's word).
 */
export function DetailSection(props: {
  title: string;
  testId: string;
  detail: DetailState;
  id: number;
  isOpenAtFirst: boolean;
  countOf: (detail: FileDetail) => number | string;
  empty: string | ((detail: FileDetail) => string);
  footer?: ReactNode;
  children: (detail: FileDetail) => ReactElement;
}): ReactElement {
  const detail = detailFor(props.detail, props.id);
  const count = detail.status === 'ready' ? props.countOf(detail.detail) : null;
  const empty =
    detail.status !== 'ready'
      ? ''
      : typeof props.empty === 'string'
        ? props.empty
        : props.empty(detail.detail);
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
        <p className="panel-empty">{empty}</p>
      ) : (
        props.children(detail.detail)
      )}
      {props.footer}
    </Section>
  );
}

/**
 * The newest commits behind the file's current lines, newest first, under the sentence that says
 * how many there are; with none, why (the history worker off, refused, or not asked yet). Closed
 * at first.
 */
export function HistorySection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title={HISTORY_TITLE}
      testId="panel-history"
      detail={detail}
      id={id}
      isOpenAtFirst={false}
      countOf={(d) => d.history_total}
      empty={historyNote}
    >
      {(d) => <HistoryList detail={d} />}
    </DetailSection>
  );
}

/**
 * The commits behind the file's lines under the sentence that says how many there are. A
 * component of its own, so its dates are written only once the section is open.
 */
function HistoryList({ detail }: { detail: FileDetail }): ReactElement {
  return (
    <>
      <p className="section-note">{historyNote(detail)}</p>
      <ul className="history-list">
        {detail.history.map((entry, index) => (
          <li key={index}>
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
    </>
  );
}

function usedText(used: boolean | null): string {
  if (used === null) return 'not known yet';
  return used ? 'then read or edited' : 'not used';
}

/**
 * The times Cortex named the file to an agent, newest first, and whether the agent used it. The
 * daemon reads the latest `MAX_TOLD`; with that many, the count says they are the latest.
 */
export function ToldSection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title="What Cortex told agents"
      testId="panel-told"
      detail={detail}
      id={id}
      isOpenAtFirst={false}
      countOf={(d) =>
        d.told.length === MAX_TOLD ? `latest ${formatCount(MAX_TOLD)}` : d.told.length
      }
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
