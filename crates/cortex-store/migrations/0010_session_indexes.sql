-- What a prompt looks up of the session it comes in: the session's tasks in order, and the edits
-- of those tasks, indexed apart from the other events so that finding them reads no other row.
CREATE INDEX tasks_session ON tasks (session_id, seq);
CREATE INDEX events_edits ON events (task_id) WHERE kind = 'edit';
