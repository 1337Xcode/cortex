// The overview's first section: the agent sessions running in the repository now, five rows at a
// time, each a button that flies to the session's newest file and explains the session on hover
// and focus. Open while a session runs; collapsed, saying so, when none does; open while the live
// view is paused, to say so, and collapsed while it first connects.
import type { ReactElement } from 'react';
import type { LiveSession } from '../api/live';
import { ClippedText } from '../components/Clipped';
import { agentIcon } from '../icons/agentIcons';
import { IconSlot } from '../icons/IconSlot';
import { agentFamily } from '../map/agents';
import { namedLiveFiles } from '../map/mapTooltip';
import type { Scene } from '../map/scene';
import { useSessionNames } from '../state/sessionNames';
import { useMapStore } from '../state/store';
import { liveSentence, sessionLead } from '../text/liveWords';
import { namesOfFiles } from '../text/pathNames';
import type { SessionNames } from '../text/sessionNames';
import { FileRow } from './FileRow';
import { RowList } from './RowList';
import { Section } from './Section';

/**
 * What leads a session's row: the diamond in its agent's colour, as the marker on the map, then
 * the agent's icon (an empty box when it has none, so the rows' names line up).
 */
function AgentMark({ agent }: { agent: string }): ReactElement {
  return (
    <span className="agent-lead" aria-hidden="true">
      <span className="agent-mark" data-agent={agentFamily(agent)} />
      <IconSlot icon={agentIcon(agent)} />
    </span>
  );
}

/** One session's row: a button to its newest file the map holds, or plain text without one. */
function SessionRow({
  session,
  scene,
  names,
}: {
  session: LiveSession;
  scene: Scene;
  names: SessionNames;
}): ReactElement {
  const files = namedLiveFiles(session, scene);
  const sentence = liveSentence(session, files, names);
  const newest = session.files.find((entry) => entry.file < scene.snapshot.files.length);
  const newestPath = newest === undefined ? '' : (scene.snapshot.files[newest.file]?.path ?? '');
  if (newest === undefined) {
    return (
      <p className="live-row">
        <AgentMark agent={session.agent} />
        <ClippedText text={sentence} kind="words" className="live-row-text" />
      </p>
    );
  }
  return (
    <FileRow
      file={newest.file}
      name={{
        lead: sessionLead(session, newest.mode, names),
        path: namesOfFiles(scene.snapshot.files).nameOf(newestPath),
      }}
      measure={null}
      sentence={sentence}
      lead={<AgentMark agent={session.agent} />}
    />
  );
}

/** What the section says instead of rows, by the live view's state. */
function emptyText(status: 'connecting' | 'paused' | 'open'): string {
  if (status === 'connecting') return 'Connecting to the live view.';
  if (status === 'paused') return 'Live view paused, reconnecting.';
  return 'No agent session is running in this repository.';
}

/** The live sessions section of the overview. */
export function LiveSessions({ scene }: { scene: Scene }): ReactElement {
  const live = useMapStore((state) => state.live);
  const names = useSessionNames();
  const sessions = live.status === 'open' ? live.state.sessions : [];
  const hasSessions = sessions.length > 0;
  return (
    <Section
      // Remounted when sessions start or stop or the live view pauses, so it opens or collapses.
      key={hasSessions ? 'running' : live.status}
      title="Live sessions"
      count={live.status === 'open' ? sessions.length : null}
      isOpenAtFirst={hasSessions || live.status === 'paused'}
      testId="overview-live"
    >
      {hasSessions ? (
        <RowList
          items={sessions}
          keyOf={(session) => session.id}
          label="Agent sessions running now"
          renderRow={(session) => <SessionRow session={session} scene={scene} names={names} />}
        />
      ) : (
        <p className="panel-empty">{emptyText(live.status)}</p>
      )}
    </Section>
  );
}
