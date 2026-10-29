//! Row ids, one type per table so that a symbol id is never passed where a node id belongs.
//!
//! Only the store makes ids; everything else gets them from the store's results.

/// Declares an id type over a table's integer row id.
macro_rules! row_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub(crate) i64);

        impl $name {
            /// The row id as stored.
            #[must_use]
            pub fn get(self) -> i64 {
                self.0
            }
        }
    };
}

row_id! {
    /// A row of `files`.
    FileId
}

row_id! {
    /// A row of `symbols`.
    SymbolId
}

row_id! {
    /// A row of `refs`.
    RefId
}

row_id! {
    /// A node of the graph: a row of `nodes`.
    NodeId
}
