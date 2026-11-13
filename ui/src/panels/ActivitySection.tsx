// The activity: what agents did lately, newest first, five rows then "Show N more". Each row is a
// sentence ("Claude Code 3f2a… edited book.rs") led by the agent's icon and the file's, over how
// long ago it was; its exact time in UTC is said on hover and focus (and to screen readers). A row
// about a file the snapshot on screen lists, at the path the daemon gave, is a button that flies
// to it; a file it does not hold yet is named by its path, and the map is asked to read its data
// again. Open while an agent session runs, as the live sessions are, unless the person chose;
// asked for when it first opens, and read again quietly with the rest of the map.
import { useCallback, useEffect, type ReactElement } from 'react';
import { fetchActivity, MAX_EVENTS, type ActivityEvent } from '../api/activity';
import { agentIcon } from '../icons/agentIcons';
import { fileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import { isOutOfStep, listedId, nameFile, namedPath, type NamedFile } from '../map/namedFile';
import type { Scene } from '../map/scene';
import { useMapStore } from '../state/store';
import { activityTimesNote, eventLine, eventSentence, shortAgo } from '../text/activityWords';
import { formatCount } from '../text/format';
import { namedFileNote, namedFileSentence, shownName } from '../text/namedFileWords';
import { distinctNames } from '../text/names';
import { LoadedBody } from './LoadedBody';
import { RowList } from './RowList';
import { Section } from './Section';
import { StackRow } from './StackRow';

/** One event: a button to its file when the snapshot on screen lists it, plain text otherwise. */
function EventRow({
  event,
  named,
  name,
  readMs,
}: {
  event: ActivityEvent;
  named: NamedFile;
  name: string | null;
  readMs: number;
}): ReactElement {
  const path = namedPath(named);
  const note = namedFileNote(named);
  const noteSentence = namedFileSentence(named);
  const sentence = eventSentence(event, name, readMs);
  return (
    <StackRow
      file={listedId(named)}
      lead={
        <>
          <IconSlot icon={event.agent === null ? null : agentIcon(event.agent.name)} />
          <IconSlot icon={path === null ? null : fileIcon(path)} />
        </>
      }
      what={eventLine(event, name)}
      meta={note === null ? [shortAgo(event.at_ms, readMs)] : [shortAgo(event.at_ms, readMs), note]}
      sentence={noteSentence === null ? sentence : `${sentence} ${noteSentence}`}
    />
  );
}

/** The events once read, against the snapshot of `scene`. */
function ActivityList({
  events,
  readMs,
  scene,
}: {
  events: readonly ActivityEvent[];
  readMs: number;
  scene: Scene;
}): ReactElement {
  const { files } = scene.snapshot;
  const noteOutOfStep = useMapStore((state) => state.noteOutOfStep);
  const isBehind = events.some((event) => isOutOfStep(files, event.file, event.path));
  useEffect(() => {
    if (isBehind) noteOutOfStep();
  }, [isBehind, noteOutOfStep, files]);
  if (events.length === 0) {
    return <p className="panel-empty">No agent has reported anything here yet.</p>;
  }
  // An id counts only where the snapshot holds the same path: ids move when files are added.
  const items = events.map((event, index) => ({
    event,
    index,
    named: nameFile(files, event.file, event.path),
  }));
  const listed = items.flatMap(({ named }) => (named.kind === 'listed' ? [named.path] : []));
  const distinct = distinctNames(listed);
  const names = new Map(listed.map((path, i) => [path, distinct[i] ?? path]));
  return (
    <>
      <p className="section-note">{activityTimesNote(readMs)}</p>
      <RowList
        items={items}
        keyOf={({ index }) => index}
        label="What agents did, newest first"
        renderRow={({ event, named }) => (
          <EventRow event={event} named={named} name={shownName(named, names)} readMs={readMs} />
        )}
      />
      <p className="section-note">
        Agents&apos; events among the {formatCount(MAX_EVENTS)} newest Cortex recorded; edits seen
        outside any agent are left out.
      </p>
    </>
  );
}

/** The overview's activity section. */
export function ActivitySection({ scene }: { scene: Scene }): ReactElement {
  const activity = useMapStore((state) => state.activity);
  const live = useMapStore((state) => state.live);
  const loadActivity = useMapStore((state) => state.loadActivity);
  const load = useCallback(() => {
    void loadActivity(fetchActivity);
  }, [loadActivity]);
  const isRunning = live.status === 'open' && live.state.sessions.length > 0;
  return (
    <Section
      title="Activity"
      count={activity.status === 'ready' ? activity.value.events.length : null}
      isOpenAtFirst={isRunning}
      // Opens while a session runs and closes after, until the person opens or closes it.
      openWhile={isRunning}
      testId="overview-activity"
    >
      <LoadedBody state={activity} load={load} what="the activity" testId="activity-error">
        {(value, readMs) => <ActivityList events={value.events} readMs={readMs} scene={scene} />}
      </LoadedBody>
    </Section>
  );
}
