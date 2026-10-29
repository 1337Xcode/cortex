//! The scopes a tags query's locals patterns find, and the names defined in each, which a pattern
//! with `#is-not? local` consults: a name defined in a scope around a use, or in one around that
//! while the scopes inherit, is local there and is not recorded. Scopes and definitions are noted in
//! the order the query finds them, so a name is local only after its definition, and the scopes
//! around a name are searched from the one noted last, as in tree-sitter's tags format.
//!
//! tree-sitter-tags searches every scope noted so far, so a name after many sibling blocks (the
//! examples of a long spec file) is checked against each of them. Here each scope also keeps the
//! scope noted before it that holds it. While no scope starts before the one noted before it, as
//! when tree-sitter yields the matches of patterns that each capture one node, a search jumps from
//! a scope that does not hold the name to that holder, over the scopes noted in between, which
//! cannot hold the name either. The answer is the one the full search gives; a scope that starts
//! before the one noted before it turns the jumps off for the rest of the file.

use std::collections::BTreeSet;

use tree_sitter::{Node, QueryCapture};

use crate::grammar::TagCapture;

/// One scope and the names defined in it so far.
#[derive(Debug)]
struct Scope<'source> {
    /// The byte offset where it starts.
    start: usize,
    /// The byte offset just past its end.
    end: usize,
    /// Whether the names of the scopes around it are local in it too.
    inherits: bool,
    /// The scope noted last before this one that holds it, by position in the list.
    holder: Option<usize>,
    /// The names defined in it, in a set, so a scope with many definitions answers at once.
    names: BTreeSet<&'source [u8]>,
}

impl Scope<'_> {
    /// Whether the scope holds the bytes from `start` to `end`.
    fn holds(&self, start: usize, end: usize) -> bool {
        self.start <= start && end <= self.end
    }
}

/// Every scope found so far in one file, the whole file first.
#[derive(Debug)]
pub(crate) struct Scopes<'source> {
    /// The file's contents.
    source: &'source [u8],
    /// The scopes noted so far, in the order they were noted.
    noted: Vec<Scope<'source>>,
    /// Whether no scope so far started before the one noted before it.
    is_in_order: bool,
}

impl<'source> Scopes<'source> {
    /// The scopes of `source` before any is found: the whole file, which inherits nothing.
    pub(crate) fn new(source: &'source [u8]) -> Self {
        Self {
            source,
            noted: vec![Scope {
                start: 0,
                end: source.len(),
                inherits: false,
                holder: None,
                names: BTreeSet::new(),
            }],
            is_in_order: true,
        }
    }

    /// Notes the scopes and definitions one match of a locals pattern captures, `meanings` saying
    /// what each capture marks; a scope it finds inherits the names around it when `inherits`.
    pub(crate) fn note(
        &mut self,
        captures: &[QueryCapture<'_>],
        meanings: &[TagCapture],
        inherits: bool,
    ) {
        for capture in captures {
            let meaning = usize::try_from(capture.index)
                .ok()
                .and_then(|index| meanings.get(index));
            let (start, end) = (capture.node.start_byte(), capture.node.end_byte());
            match meaning {
                Some(TagCapture::LocalScope) => self.add_scope(start, end, inherits),
                Some(TagCapture::LocalDefinition) => self.define(start, end),
                _ => {}
            }
        }
    }

    /// Adds the scope from `start` to `end`, after the scopes noted so far.
    fn add_scope(&mut self, start: usize, end: usize, inherits: bool) {
        self.is_in_order &= self.noted.last().is_none_or(|last| last.start <= start);
        let holder = self.holder_before(self.noted.len(), start, end);
        self.noted.push(Scope {
            start,
            end,
            inherits,
            holder,
            names: BTreeSet::new(),
        });
    }

    /// Records the name from `start` to `end` in the scope noted last that holds it.
    fn define(&mut self, start: usize, end: usize) {
        let Some(name) = self.source.get(start..end) else {
            return;
        };
        let holder = self.holder_before(self.noted.len(), start, end);
        if let Some(scope) = holder.and_then(|index| self.noted.get_mut(index)) {
            scope.names.insert(name);
        }
    }

    /// Whether the name `node` holds is defined in a scope around it, searching outward from the
    /// scope noted last until a scope that does not inherit.
    pub(crate) fn is_local(&self, node: Node<'_>) -> bool {
        self.is_local_at(node.start_byte(), node.end_byte())
    }

    /// Whether the name from `start` to `end` is defined in a scope around it.
    fn is_local_at(&self, start: usize, end: usize) -> bool {
        let Some(name) = self.source.get(start..end) else {
            return false;
        };
        let mut holder = self.holder_before(self.noted.len(), start, end);
        while let Some(index) = holder {
            let Some(scope) = self.noted.get(index) else {
                return false;
            };
            if scope.names.contains(name) {
                return true;
            }
            if !scope.inherits {
                return false;
            }
            holder = self.holder_before(index, start, end);
        }
        false
    }

    /// The scope noted last before position `limit` in the list that holds the bytes from `start`
    /// to `end`.
    fn holder_before(&self, limit: usize, start: usize, end: usize) -> Option<usize> {
        let mut at = limit.checked_sub(1);
        while let Some(index) = at {
            let scope = self.noted.get(index)?;
            if scope.holds(start, end) {
                return Some(index);
            }
            // In order, a scope noted between this one and its holder either ended before this
            // one starts or lies inside it; as the bytes start no earlier than this scope and are
            // not inside it, no such scope holds them either.
            at = if self.is_in_order && scope.start <= start {
                scope.holder
            } else {
                index.checked_sub(1)
            };
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use proptest::sample::Index;

    use super::*;

    /// One thing a query reports, by byte range: a scope, a definition, or a name to look up.
    #[derive(Debug, Clone, Copy)]
    enum Step {
        /// A scope, which inherits the names around it or not.
        Scope(usize, usize, bool),
        /// A definition.
        Define(usize, usize),
        /// A name to look up.
        Look(usize, usize),
    }

    /// The search tree-sitter-tags makes: every scope noted so far, from the last, each with its
    /// names in a list.
    struct EveryScope<'source> {
        /// The file's contents.
        source: &'source [u8],
        /// Each scope's start, end, whether it inherits, and its names.
        scopes: Vec<(usize, usize, bool, Vec<&'source [u8]>)>,
    }

    impl<'source> EveryScope<'source> {
        /// The whole file as the one scope, which inherits nothing.
        fn new(source: &'source [u8]) -> Self {
            Self {
                source,
                scopes: vec![(0, source.len(), false, Vec::new())],
            }
        }

        /// Applies `step`, giving the answer to a look-up.
        fn apply(&mut self, step: Step) -> Option<bool> {
            match step {
                Step::Scope(start, end, inherits) => {
                    self.scopes.push((start, end, inherits, Vec::new()));
                    None
                }
                Step::Define(start, end) => {
                    let name = self.source.get(start..end)?;
                    let holder = self
                        .scopes
                        .iter_mut()
                        .rev()
                        .find(|scope| scope.0 <= start && end <= scope.1);
                    holder?.3.push(name);
                    None
                }
                Step::Look(start, end) => {
                    let name = self.source.get(start..end)?;
                    for scope in self.scopes.iter().rev() {
                        if !(scope.0 <= start && end <= scope.1) {
                            continue;
                        }
                        if scope.3.contains(&name) {
                            return Some(true);
                        }
                        if !scope.2 {
                            return Some(false);
                        }
                    }
                    Some(false)
                }
            }
        }
    }

    /// A random file and the steps a query would report for it. Each token is one of: open a
    /// scope that inherits (0) or not (1), both taking no bytes; close the innermost open scope
    /// (2); use the one-byte name `a`, `b` or `c` (3 to 5); define `a` or `b` (6, 7); or use an
    /// empty name (8). Scopes still open at the end close there. The steps come in the file's
    /// order, scopes at their start; each delay then moves one step later by a few places, as a
    /// pattern that ends past its capture would.
    fn file_and_steps(tokens: &[u8], delays: &[(Index, usize)]) -> (Vec<u8>, Vec<Step>) {
        let (mut source, mut steps, mut open) = (Vec::new(), Vec::new(), Vec::new());
        for token in tokens {
            let at = source.len();
            match token {
                0 | 1 => {
                    open.push(steps.len());
                    steps.push(Step::Scope(at, at, *token == 0));
                }
                2 => close(&mut steps, open.pop(), at),
                3..=7 => {
                    source.push(b"abcab"[usize::from(token - 3)]);
                    let range = (at, at + 1);
                    steps.push(if *token < 6 {
                        Step::Look(range.0, range.1)
                    } else {
                        Step::Define(range.0, range.1)
                    });
                }
                _ => steps.push(Step::Look(at, at)),
            }
        }
        while let Some(position) = open.pop() {
            close(&mut steps, Some(position), source.len());
        }
        for (index, distance) in delays {
            if steps.is_empty() {
                break;
            }
            let from = index.index(steps.len());
            let to = (from + distance).min(steps.len() - 1);
            let step = steps.remove(from);
            steps.insert(to, step);
        }
        (source, steps)
    }

    /// Sets the end of the scope opened at `position` in `steps`.
    fn close(steps: &mut [Step], position: Option<usize>, end: usize) {
        if let Some(Step::Scope(_, scope_end, _)) = position.and_then(|at| steps.get_mut(at)) {
            *scope_end = end;
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1024))]

        /// The jumping search gives every look-up the answer the search of every scope gives, for
        /// scopes, definitions and names in the file's order or out of it.
        #[test]
        fn answers_match_a_search_of_every_scope(
            tokens in prop::collection::vec(0u8..9, 0..160),
            delays in prop::collection::vec((any::<Index>(), 0usize..6), 0..4),
        ) {
            let (source, steps) = file_and_steps(&tokens, &delays);
            let mut scopes = Scopes::new(&source);
            let mut every = EveryScope::new(&source);
            for step in steps {
                let expected = every.apply(step);
                match step {
                    Step::Scope(start, end, inherits) => scopes.add_scope(start, end, inherits),
                    Step::Define(start, end) => scopes.define(start, end),
                    Step::Look(start, end) => {
                        prop_assert_eq!(Some(scopes.is_local_at(start, end)), expected);
                    }
                }
            }
        }
    }
}
