// The daemon's health, as `cortex status` shows it: its version and first reading, how long it has
// run and what it lost, each hook's handling times in a small table, its newest errors, the history
// miner, the graph's size, upkeep's last run and the history worker. The last section of the
// overview, closed at first: a person needs it when something looks wrong, not on every visit, and
// a header chip would take room the search box and the summary need on a narrow screen. Its count
// says how many errors the daemon kept once it is read; it is asked for when it opens, read again
// on request and with the map's quiet refresh, and says when it was read.
import { useCallback, type ReactElement } from 'react';
import { visibleText } from '../api/guard';
import { fetchHealth, type DaemonHealth, type DaemonReport, type Health } from '../api/health';
import { Button } from '../components/ui/button';
import { useMapStore } from '../state/store';
import { formatCount, formatMicros, formatUtcTime } from '../text/format';
import {
  errorCount,
  errorSentence,
  gitSentence,
  graphSentence,
  HISTORY_SENTENCES,
  HOOK_TIMES_NOTE,
  hookName,
  runSentence,
  versionSentence,
} from '../text/healthWords';
import { LoadedBody } from './LoadedBody';
import { Section } from './Section';

/** Each hook's handling times, in microseconds as the daemon measured them. */
function HookTable({ daemon }: { daemon: DaemonHealth }): ReactElement {
  if (daemon.hooks.length === 0) {
    return <p className="section-note">No hook has sent a request since the daemon started.</p>;
  }
  return (
    <table className="health-table" data-testid="health-hooks">
      <caption className="section-note">{HOOK_TIMES_NOTE}</caption>
      <thead>
        <tr>
          <th scope="col">Hook</th>
          <th scope="col">Requests</th>
          <th scope="col">Median</th>
          <th scope="col">95th</th>
          <th scope="col">99th</th>
        </tr>
      </thead>
      <tbody>
        {daemon.hooks.map((hook) => (
          <tr key={hook.hook}>
            <th scope="row">{hookName(hook.hook)}</th>
            <td>{formatCount(hook.requests)}</td>
            <td>{formatMicros(hook.p50_us)}</td>
            <td>{formatMicros(hook.p95_us)}</td>
            <td>{formatMicros(hook.p99_us)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** What the daemon reported, part by part. */
function DaemonParts({ daemon }: { daemon: DaemonHealth }): ReactElement {
  return (
    <>
      <p className="receipts-line">{runSentence(daemon)}</p>
      <HookTable daemon={daemon} />
      <div className="panel-kind">
        <h4>Errors</h4>
        {daemon.errors.length === 0 ? (
          <p className="panel-empty">No error since the daemon started.</p>
        ) : (
          <ul className="health-errors" aria-label="The daemon's newest errors, newest first">
            {daemon.errors.map((error, index) => (
              <li key={index}>{errorSentence(error)}</li>
            ))}
          </ul>
        )}
      </div>
      <p className="receipts-line">{gitSentence(daemon)}</p>
      <p className="receipts-line">{graphSentence(daemon)}</p>
      <p className="receipts-line">{HISTORY_SENTENCES[daemon.history]}</p>
    </>
  );
}

/** The daemon's report: its parts, or why there are none to show. */
function DaemonReportParts({ report }: { report: DaemonReport }): ReactElement {
  switch (report.status) {
    case 'read':
      return <DaemonParts daemon={report.health} />;
    case 'absent':
      return (
        <p className="panel-empty">
          The daemon could not say how it is doing; it may be stopping. Read it again in a moment.
        </p>
      );
    case 'unreadable':
      return (
        <p className="panel-empty" role="alert" data-testid="health-unreadable">
          The daemon&apos;s report could not be read ({report.error.code}):{' '}
          {visibleText(report.error.message)}. Run cortex status in a terminal to see it.
        </p>
      );
  }
}

/** The health once read at `readMs`, with the button that reads it again. */
function HealthParts({ health, readMs }: { health: Health; readMs: number }): ReactElement {
  const loadHealth = useMapStore((state) => state.loadHealth);
  return (
    <>
      <p className="receipts-line" data-testid="health-version">
        {versionSentence(health)}
      </p>
      <DaemonReportParts report={health.daemon} />
      <p className="section-note">
        Read at {formatUtcTime(readMs)} UTC, by this computer&apos;s clock.
      </p>
      <Button
        variant="ghost"
        size="compact"
        className="show-more -ml-sm text-muted-foreground hover:text-foreground"
        onClick={() => {
          void loadHealth(fetchHealth, { isQuiet: true });
        }}
      >
        Read again
      </Button>
    </>
  );
}

/** The overview's health section. */
export function HealthSection(): ReactElement {
  const health = useMapStore((state) => state.health);
  const loadHealth = useMapStore((state) => state.loadHealth);
  const load = useCallback(() => {
    void loadHealth(fetchHealth);
  }, [loadHealth]);
  return (
    <Section
      title="Health"
      count={health.status === 'ready' ? errorCount(health.value) : null}
      isOpenAtFirst={false}
      testId="overview-health"
    >
      <LoadedBody state={health} load={load} what="the daemon's health" testId="health-error">
        {(value, readMs) => <HealthParts health={value} readMs={readMs} />}
      </LoadedBody>
    </Section>
  );
}
