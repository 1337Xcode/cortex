//! The names coordination stores: what a session did to a node of the map, which decides how long
//! its position there holds.

use crate::named::named_kind;

named_kind! {
    /// What a session did to a node: the mode of its position there.
    PositionMode {
        /// It read the node.
        Read => "read",
        /// It edited the node.
        Edit => "edit",
        /// It claimed the node, as an advisory hold.
        Claim => "claim",
    }
}

/// The working tree of a repository a session works in: the main one, or one linked to it (`git
/// worktree add`), by git's name for it when that could be read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorkingTree {
    /// The repository's main working tree.
    Main,
    /// A linked working tree, by git's name for it when that could be read as a name.
    Linked(Option<crate::Identifier>),
}
