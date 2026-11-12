// The side panel's sections for one file, each with its own empty state: its numbers, its
// strongest links by kind, the facts anchored to it (suspect ones marked), the stored history of
// its commits, and what Cortex told agents about it with whether they then used it.
import { useId, type ReactElement } from 'react';
import type { FactStatus, FileDetail, ToldKind } from '../api/file';
import { LINK_KINDS, type GraphFile } from '../api/graph';
import { visibleText } from '../api/guard';
import { LINK_KIND_NAMES } from '../controls/Legend';
import type { Scene } from '../map/scene';
import { useMapStore, type DetailState } from '../state/store';
import { countOf, formatCount, formatDate } from '../text/format';
import { distinctNames } from '../text/names';
import { PANEL_LINKS_PER_KIND } from '../theme/tokens';
import { strongestLinks, type PanelLink } from './fileLinks';

const STATUS_NAMES: Readonly<Record<FactStatus, string>> = {
  active: 'Active',
  candidate: 'Candidate',
  suspect: 'Suspect',
};

const TOLD_NAMES: Readonly<Record<ToldKind, string>> = {
  work_order: 'Work order',
  warning: 'Warning',
};

/** What the attributed tokens count, for the panel's description of that number. */
export const TOKENS_ATTRIBUTED_MEANING =
  'Tokens of agent turns whose last read or edit was this file.';

/** The file's counts as a definition list; each name says exactly what it counts. */
export function NumbersSection({ file }: { file: GraphFile }): ReactElement {
  const descriptionId = useId();
  const rows: [string, number, string | null][] = [
    ['Lines', file.lines, null],
    ['Agent reads', file.reads, null],
    ['Commits', file.commits, null],
    ['Tokens attributed', file.tokens, descriptionId],
  ];
  return (
    <>
      <dl className="panel-numbers">
        {rows.map(([name, value, described]) => (
          <div key={name} title={described === null ? undefined : TOKENS_ATTRIBUTED_MEANING}>
            <dt>{name}</dt>
            <dd aria-describedby={described ?? undefined}>{formatCount(value)}</dd>
          </div>
        ))}
      </dl>
      <p id={descriptionId} className="visually-hidden">
        {TOKENS_ATTRIBUTED_MEANING}
      </p>
    </>
  );
}

function LinkRow(props: { link: PanelLink; name: string; isCoChange: boolean }): ReactElement {
  const { link, name, isCoChange } = props;
  const selectFile = useMapStore((state) => state.selectFile);
  const measure = isCoChange
    ? countOf(link.commits, 'commit')
    : `strength ${link.weight.toFixed(2)}`;
  return (
    <li>
      <button
        type="button"
        className="link-button"
        title={link.path}
        onClick={() => {
          selectFile(link.other, { fly: true });
        }}
      >
        {name}
      </button>
      <span className="panel-muted">{measure}</span>
    </li>
  );
}

/** The file's strongest links of each kind. */
export function LinksSection({ scene, id }: { scene: Scene; id: number }): ReactElement {
  const grouped = strongestLinks(scene, id, PANEL_LINKS_PER_KIND);
  // Names are told apart across every list the panel shows at once.
  const shown = LINK_KINDS.flatMap((kind) => grouped[kind].map((link) => link.path));
  const names = new Map(shown.map((path, i) => [path, distinctNames(shown)[i] ?? path]));
  return (
    <section className="panel-section" aria-labelledby="panel-links">
      <h3 id="panel-links">Linked files</h3>
      {LINK_KINDS.map((kind) => (
        <div key={kind} className="panel-kind" data-testid={`panel-links-${kind}`}>
          <h4>{LINK_KIND_NAMES[kind]}</h4>
          {grouped[kind].length === 0 ? (
            <p className="panel-empty">None learned yet.</p>
          ) : (
            <ul className="panel-list">
              {grouped[kind].map((link) => (
                <LinkRow
                  key={link.other}
                  link={link}
                  name={names.get(link.path) ?? link.path}
                  isCoChange={kind === 'co_change'}
                />
              ))}
            </ul>
          )}
        </div>
      ))}
    </section>
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

function DetailBody({
  detail: stored,
  id,
  children,
}: {
  detail: DetailState;
  id: number;
  children: (detail: FileDetail) => ReactElement;
}): ReactElement {
  const detail = detailFor(stored, id);
  if (detail.status === 'ready') return children(detail.detail);
  if (detail.status === 'error') {
    return (
      <p className="panel-empty" role="alert">
        Could not load this ({detail.error.code}): {detail.error.message}
      </p>
    );
  }
  return <p className="panel-empty">Loading.</p>;
}

/** One section fed by the file's detail, with its title, test id and empty text. */
function DetailSection(props: {
  title: string;
  testId: string;
  detail: DetailState;
  id: number;
  isEmpty: (detail: FileDetail) => boolean;
  empty: string;
  children: (detail: FileDetail) => ReactElement;
}): ReactElement {
  return (
    <section className="panel-section" data-testid={props.testId} aria-label={props.title}>
      <h3>{props.title}</h3>
      <DetailBody detail={props.detail} id={props.id}>
        {(detail) =>
          props.isEmpty(detail) ? (
            <p className="panel-empty">{props.empty}</p>
          ) : (
            props.children(detail)
          )
        }
      </DetailBody>
    </section>
  );
}

/** The facts anchored to the file; suspect ones are marked in words and colour. */
export function FactsSection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title="Facts"
      testId="panel-facts"
      detail={detail}
      id={id}
      isEmpty={(d) => d.facts.length === 0}
      empty="No facts are recorded for this file."
    >
      {(d) => (
        <ul className="panel-list panel-facts">
          {d.facts.map((fact) => (
            <li key={fact.id} data-status={fact.status}>
              <span className="fact-status">{STATUS_NAMES[fact.status]}</span>
              <span className="panel-muted">{visibleText(fact.kind)}</span>
              <p className="fact-text">{visibleText(fact.text)}</p>
            </li>
          ))}
        </ul>
      )}
    </DetailSection>
  );
}

/** The stored commit subjects that touched the file, newest first. */
export function HistorySection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title="History"
      testId="panel-history"
      detail={detail}
      id={id}
      isEmpty={(d) => d.history.length === 0}
      empty="No commit history read for this file yet; Cortex fills it in as it reads the repository's history."
    >
      {(d) => (
        <ul className="panel-list">
          {d.history.map((entry, index) => (
            <li key={index}>
              <span className="history-subject">{visibleText(entry.subject)}</span>
              <span className="panel-muted">
                {formatDate(entry.date_s * 1000)}
                {entry.refs === '' ? '' : ` ${visibleText(entry.refs)}`}
              </span>
            </li>
          ))}
        </ul>
      )}
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
      isEmpty={(d) => d.told.length === 0}
      empty="Cortex has not named this file to an agent yet."
    >
      {(d) => (
        <ul className="panel-list">
          {d.told.map((entry, index) => (
            <li key={index} data-used={String(entry.used)}>
              <span>{TOLD_NAMES[entry.kind]}</span>
              <span className="panel-muted">
                {formatDate(entry.at_ms)}, {usedText(entry.used)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </DetailSection>
  );
}
