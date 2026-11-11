//! The pairs each recent task was warned of, kept from the moment its warnings are decided. The
//! store learns of a warning only once the writer records it, after the answer is written; two
//! edits answered close together, or one answered while the writer is busy, would otherwise both
//! find the pair untold and both tell it. Deciding under this lock and keeping what was decided in
//! the same step makes each pair told once a task, whatever the writer is doing.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Mutex, PoisonError};

use cortex_core::{Ulid, WarningKind};
use cortex_retrieve::Warned;
use cortex_store::NodeId;

/// One pair a warning named: its kind, what it was about, and the file it named.
pub(crate) type Pair = (WarningKind, NodeId, NodeId);

/// How many tasks' pairs are kept, the task decided for longest ago dropped first: tasks run one
/// at a time in a session, so a few dozen cover every session working at once. A task dropped
/// still has the pairs the store recorded.
const MAX_TASKS: usize = 64;

/// The pairs each recent task was warned of.
#[derive(Debug, Default)]
pub(crate) struct Told {
    /// Each task with its pairs, the one decided for longest ago first.
    tasks: Mutex<VecDeque<(Ulid, BTreeSet<Pair>)>>,
}

impl Told {
    /// Runs `decide` with the pairs task `task_id` was told so far, and keeps the pairs of the
    /// warnings it gives as told, all while no other decision for any task runs.
    pub(crate) fn deciding(
        &self,
        task_id: Ulid,
        decide: impl FnOnce(&BTreeSet<Pair>) -> Warned,
    ) -> Warned {
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        let at = match tasks.iter().position(|(task, _)| *task == task_id) {
            Some(at) => at,
            None => {
                if tasks.len() == MAX_TASKS {
                    tasks.pop_front();
                }
                tasks.push_back((task_id, BTreeSet::new()));
                tasks.len() - 1
            }
        };
        let Some((_, told)) = tasks.get_mut(at) else {
            return Warned::default();
        };
        let warned = decide(told);
        told.extend(pairs_of(&warned));
        warned
    }

    /// Whether task `task_id` was told a warning of `kind`, recorded or not yet.
    pub(crate) fn has_told(&self, task_id: Ulid, kind: WarningKind) -> bool {
        let tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        tasks.iter().any(|(task, told)| {
            *task == task_id && told.iter().any(|(told_kind, _, _)| *told_kind == kind)
        })
    }

    /// Forgets that task `task_id` was told `pairs`: its warnings never reached the agent.
    pub(crate) fn forget(&self, task_id: Ulid, pairs: &[Pair]) {
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, told)) = tasks.iter_mut().find(|(task, _)| *task == task_id) {
            for pair in pairs {
                told.remove(pair);
            }
        }
    }
}

/// The pairs the warnings of `warned` name: one for each file each names.
pub(crate) fn pairs_of(warned: &Warned) -> Vec<Pair> {
    warned
        .given
        .iter()
        .flat_map(|warning| {
            warning
                .targets
                .iter()
                .map(|target| (warning.kind, warning.source, *target))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cortex_retrieve::Warning;

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// What a decision that warns of `kind` about `source`, naming `target`, gives.
    fn warned(kind: WarningKind, source: NodeId, target: NodeId) -> Warned {
        Warned {
            given: vec![Warning {
                kind,
                source,
                targets: vec![target],
                strength: 1.0,
                line: "a warning".to_owned(),
            }],
            text: Some("[cortex] a warning\n".to_owned()),
        }
    }

    /// A task's second decision sees the pair its first gave; another task's sees none of it.
    #[test]
    fn a_decision_sees_what_its_task_was_told() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let (a, b) = (
            add_file(&mut store, "a.py", 1)?,
            add_file(&mut store, "b.py", 1)?,
        );
        let (first, second) = (Ulid::from_seed(1, b"task"), Ulid::from_seed(2, b"task"));
        let told = Told::default();
        let mut seen = Vec::new();
        told.deciding(first, |pairs| {
            seen.push(pairs.clone());
            warned(WarningKind::Test, a, b)
        });
        told.deciding(first, |pairs| {
            seen.push(pairs.clone());
            Warned::default()
        });
        told.deciding(second, |pairs| {
            seen.push(pairs.clone());
            Warned::default()
        });
        let pair = (WarningKind::Test, a, b);
        assert_eq!(
            seen,
            [BTreeSet::new(), BTreeSet::from([pair]), BTreeSet::new()]
        );
        Ok(())
    }

    /// A task told an origin note has told it, of that kind only; another task has not, and a task
    /// whose note was forgotten (it never reached the agent) has not either.
    #[test]
    fn a_task_says_which_kinds_it_was_told() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let a = add_file(&mut store, "a.py", 1)?;
        let (first, second) = (Ulid::from_seed(1, b"task"), Ulid::from_seed(2, b"task"));
        let told = Told::default();
        told.deciding(first, |_| warned(WarningKind::Origin, a, a));
        assert!(told.has_told(first, WarningKind::Origin));
        assert!(!told.has_told(first, WarningKind::Test));
        assert!(!told.has_told(second, WarningKind::Origin));
        told.forget(first, &[(WarningKind::Origin, a, a)]);
        assert!(!told.has_told(first, WarningKind::Origin));
        Ok(())
    }

    /// Past 64 tasks, the one decided for longest ago is forgotten: its pairs are not seen again.
    #[test]
    fn the_oldest_task_is_forgotten_past_the_cap() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let (a, b) = (
            add_file(&mut store, "a.py", 1)?,
            add_file(&mut store, "b.py", 1)?,
        );
        let told = Told::default();
        let task = |at: u64| Ulid::from_seed(at, b"task");
        told.deciding(task(0), |_| warned(WarningKind::Test, a, b));
        for at in 1..=u64::try_from(MAX_TASKS)? {
            told.deciding(task(at), |_| Warned::default());
        }
        let mut seen = None;
        told.deciding(task(0), |pairs| {
            seen = Some(pairs.len());
            Warned::default()
        });
        assert_eq!(seen, Some(0));
        Ok(())
    }
}
