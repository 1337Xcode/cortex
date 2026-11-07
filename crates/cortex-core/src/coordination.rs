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
