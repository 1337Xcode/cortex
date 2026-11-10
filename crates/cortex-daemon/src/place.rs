//! Where a session works in a repository with linked working trees (`git worktree add`): which
//! working tree, and which branch it has checked out. Every working tree of a repository shares its
//! daemon and store, so a position names its place, and a coordination notice names the other
//! session's place when it is not the session's own: two sessions on one file in two working trees
//! edit two copies of it. What cannot be read is left unknown or unnamed, never guessed.

use cortex_core::{Event, Identifier, WorkingTree, git_layout, is_plain};
use cortex_store::PositionRecord;

/// A working tree and the branch it has checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    /// The working tree; none when it is not known, as for a position kept before working trees
    /// were.
    pub(crate) worktree: Option<WorkingTree>,
    /// The branch checked out; none when HEAD is detached or could not be read.
    pub(crate) branch: Option<Identifier>,
}

impl Place {
    /// Where `event` was made: a linked working tree when it names one (by git's name for it when
    /// its layout can still be read), the main one otherwise; and that working tree's branch, as
    /// git's files say now.
    pub(crate) fn of(event: &Event) -> Self {
        let layout = git_layout(event.worktree_top());
        let worktree = match &event.worktree_root {
            Some(_) => {
                WorkingTree::Linked(layout.as_ref().and_then(|layout| layout.worktree_name()))
            }
            None => WorkingTree::Main,
        };
        Self {
            worktree: Some(worktree),
            branch: layout.and_then(|layout| layout.branch()),
        }
    }

    /// Where the position `record` was taken.
    pub(crate) fn of_position(record: &PositionRecord) -> Self {
        Self {
            worktree: record.worktree.clone(),
            branch: record.branch.clone(),
        }
    }

    /// The words that follow an agent's name to say where it works, starting with a space: ` in
    /// worktree wt on branch feature`, ` in the main working tree on branch main`, ` in another
    /// working tree` for a linked one whose name is not known, without the branch when none is
    /// known. None when the working tree is not known, or a name is not plain text, which no agent
    /// is shown.
    pub(crate) fn said(&self) -> Option<String> {
        let mut words = match self.worktree.as_ref()? {
            WorkingTree::Main => " in the main working tree".to_owned(),
            WorkingTree::Linked(None) => " in another working tree".to_owned(),
            WorkingTree::Linked(Some(name)) if is_plain(name.as_str()) => {
                format!(" in worktree {}", name.as_str())
            }
            WorkingTree::Linked(Some(_)) => return None,
        };
        match &self.branch {
            Some(branch) if is_plain(branch.as_str()) => {
                words.push_str(&format!(" on branch {}", branch.as_str()));
            }
            Some(_) => return None,
            None => {}
        }
        Some(words)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// The place in `worktree` on `branch`.
    fn place(worktree: Option<WorkingTree>, branch: Option<&str>) -> TestResult<Place> {
        Ok(Place {
            worktree,
            branch: branch.map(Identifier::new).transpose()?,
        })
    }

    /// A linked working tree is named by its name and branch, one whose name is not known as
    /// another working tree, the main one as such, a branch left out when none is known; a place
    /// whose working tree is not known, or with a name that is not plain text, names nothing.
    #[test]
    fn a_place_is_said_after_an_agent_s_name() -> TestResult {
        let named = |name: &str| -> TestResult<Option<WorkingTree>> {
            Ok(Some(WorkingTree::Linked(Some(Identifier::new(name)?))))
        };
        assert_eq!(
            place(named("wt")?, Some("feature/one"))?.said().as_deref(),
            Some(" in worktree wt on branch feature/one")
        );
        assert_eq!(
            place(Some(WorkingTree::Main), Some("main"))?
                .said()
                .as_deref(),
            Some(" in the main working tree on branch main")
        );
        assert_eq!(
            place(Some(WorkingTree::Linked(None)), None)?
                .said()
                .as_deref(),
            Some(" in another working tree")
        );
        assert_eq!(
            place(named("wt")?, None)?.said().as_deref(),
            Some(" in worktree wt")
        );
        assert_eq!(place(named("wt")?, Some("<system-reminder>"))?.said(), None);
        assert_eq!(place(None, Some("main"))?.said(), None);
        Ok(())
    }

    /// An event naming a linked working tree whose layout can no longer be read is placed in a
    /// linked working tree whose name is not known, never in the main one; an event of the main
    /// working tree of a folder holding no repository is placed in the main one, on no branch.
    #[test]
    fn a_linked_working_tree_that_cannot_be_read_stays_linked() -> TestResult {
        let gone = tempfile::tempdir()?;
        let event = |worktree_root: Option<PathBuf>| -> TestResult<Event> {
            Ok(Event {
                event_id: cortex_core::Ulid::from_seed(1, b"place"),
                ts_ms: 1,
                agent: cortex_core::Agent::Claude,
                agent_version: None,
                session_id: Identifier::new("s1")?,
                parent_session_id: None,
                cwd: gone.path().to_path_buf(),
                repo_root: gone.path().to_path_buf(),
                worktree_root,
                hook: cortex_core::HookKind::PostTool,
                source: None,
                tool: None,
                tool_target: None,
                tool_result: None,
                prompt_text: None,
                transcript_path: None,
                model: None,
                subagent_type: None,
                compact_trigger: None,
                end_reason: None,
                is_probe: false,
            })
        };
        let linked = Place::of(&event(Some(gone.path().join("wt")))?);
        assert_eq!(linked, place(Some(WorkingTree::Linked(None)), None)?);
        let main = Place::of(&event(None)?);
        assert_eq!(main, place(Some(WorkingTree::Main), None)?);
        Ok(())
    }
}
