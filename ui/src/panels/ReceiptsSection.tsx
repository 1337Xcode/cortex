// The receipts: what agents' work in this repository cost and whether Cortex helped, as
// `cortex receipts` reports it. A section of the overview, closed at first and loaded the first
// time it opens, so the map asks the daemon for receipts only when the person wants them. It says,
// in order: the headline (the holdout comparison with its intervals and sample sizes once each arm
// has enough tasks that ended, else that there is not enough data yet and how far off it is, with
// no difference, share or saving at all), how precise the work orders and warnings were, a chart of
// tokens and turns per task, each recent session with what Cortex injected against its input, and
// the price table every cost is estimated from.
import { useEffect, type ReactElement } from 'react';
import { fetchReceipts, type Receipts, type SessionReceipt } from '../api/receipts';
import { Button } from '../components/ui/button';
import { agentIcon } from '../icons/agentIcons';
import { IconSlot } from '../icons/IconSlot';
import { useMapStore } from '../state/store';
import { countOf } from '../text/format';
import {
  compactionsSentence,
  headline,
  HOLDS_ZERO_NOTE,
  noSessionSentence,
  precisionSentence,
  pricesSentence,
  sessionOverhead,
  sessionStarted,
  sessionTitle,
  sessionTotal,
  warningSentence,
} from '../text/receiptWords';
import { ReceiptsChart } from './ReceiptsChart';
import { RowList } from './RowList';
import { Section } from './Section';

/** The headline: the comparison, or how far the tasks are from being enough for one. */
function Headline({ receipts }: { receipts: Receipts }): ReactElement {
  const { holdout } = receipts.recorded;
  const said = headline(holdout);
  if (said.kind === 'not-enough') {
    return (
      <div className="receipts-headline" data-testid="receipts-headline">
        <p className="receipts-lead">{said.text}</p>
        <p className="section-note">
          Cortex compares the tasks it held out with the rest once each side has{' '}
          {countOf(holdout.needed_per_arm, 'task')} that ended.
        </p>
      </div>
    );
  }
  return (
    <div className="receipts-headline" data-testid="receipts-headline">
      <p className="receipts-lead">{said.text}</p>
      <ul className="measure-list" aria-label="The measures compared">
        {said.measures.map((measure) => (
          <li key={measure.name} data-testid="receipts-measure">
            <span className="measure-name">{measure.name}</span>
            <span className="measure-values">{measure.arms}</span>
            <span className="measure-values">{measure.difference}</span>
            {measure.holdsZero ? <span className="measure-note">{HOLDS_ZERO_NOTE}</span> : null}
          </li>
        ))}
      </ul>
      <p className="section-note">{said.medians}</p>
    </div>
  );
}

/** One session's receipt: its name, when it started, its total, and what Cortex injected. */
function SessionRow({ session }: { session: SessionReceipt }): ReactElement {
  const compactions = compactionsSentence(session);
  return (
    <div className="receipt-row">
      <p className="receipt-title">
        <IconSlot icon={agentIcon(session.agent)} />
        <span>{sessionTitle(session)}</span>
      </p>
      <p className="receipt-meta">{sessionStarted(session)}</p>
      <p>{sessionTotal(session)}</p>
      <p data-testid="receipts-overhead">{sessionOverhead(session)}</p>
      {compactions === null ? null : <p className="receipt-meta">{compactions}</p>}
    </div>
  );
}

/** The receipts once loaded. */
function ReceiptsReady({ receipts }: { receipts: Receipts }): ReactElement {
  const { recorded, window_days: days } = receipts;
  const newestFirst = [...recorded.sessions].reverse();
  return (
    <>
      <Headline receipts={receipts} />
      <p className="receipts-line" data-testid="receipts-precision">
        {precisionSentence(recorded.precision)}
      </p>
      <p className="receipts-line" data-testid="receipts-warnings">
        {warningSentence(recorded.warnings)}
      </p>
      {recorded.sessions.length === 0 ? (
        <p className="panel-empty" data-testid="receipts-empty">
          {noSessionSentence(days)}
        </p>
      ) : (
        <>
          <div className="panel-kind receipts-part">
            <h4>Tokens and turns per task</h4>
            <ReceiptsChart sessions={recorded.sessions} />
          </div>
          <div className="panel-kind receipts-part">
            <h4>Sessions of the last {countOf(days, 'day')}</h4>
            <RowList
              items={newestFirst}
              keyOf={(session) => session.session_id}
              label={`Sessions of the last ${countOf(days, 'day')}, newest first`}
              renderRow={(session) => <SessionRow session={session} />}
            />
          </div>
        </>
      )}
      <p className="section-note" data-testid="receipts-prices">
        {pricesSentence(receipts.prices)}
      </p>
    </>
  );
}

/** The section's body: asks for the receipts once, then shows them, the wait or the failure. */
function ReceiptsBody(): ReactElement {
  const state = useMapStore((store) => store.receipts);
  const loadReceipts = useMapStore((store) => store.loadReceipts);
  useEffect(() => {
    if (state.status === 'idle') void loadReceipts(fetchReceipts);
  }, [state.status, loadReceipts]);
  switch (state.status) {
    case 'idle':
    case 'loading':
      return <p className="panel-empty">Loading the receipts.</p>;
    case 'error':
      return (
        <>
          <p className="panel-empty" role="alert" data-testid="receipts-error">
            The receipts could not be loaded ({state.error.code}): {state.error.message}. Check that
            Cortex is still running (cortex status in a terminal), then try again.
          </p>
          <Button
            variant="ghost"
            size="compact"
            className="show-more -ml-sm text-muted-foreground hover:text-foreground"
            onClick={() => {
              void loadReceipts(fetchReceipts);
            }}
          >
            Try again
          </Button>
        </>
      );
    case 'ready':
      return <ReceiptsReady receipts={state.receipts} />;
  }
}

/** The overview's receipts section: its count is the sessions shown, once they are known. */
export function ReceiptsSection(): ReactElement {
  const receipts = useMapStore((store) => store.receipts);
  return (
    <Section
      title="Receipts"
      count={receipts.status === 'ready' ? receipts.receipts.recorded.sessions.length : null}
      isOpenAtFirst={false}
      testId="overview-receipts"
    >
      <ReceiptsBody />
    </Section>
  );
}
