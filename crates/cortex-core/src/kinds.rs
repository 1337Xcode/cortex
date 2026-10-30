//! The kinds the index records, with the names and numbers storage uses for them.
//!
//! These values are written to the database, so each name and number is fixed once chosen.

/// Declares an enum whose variants each have one stored name, with `ALL`, `name` and
/// `from_name`.
macro_rules! named_kind {
    (
        $(#[$meta:meta])*
        $kind:ident { $( $(#[$variant_meta:meta])* $variant:ident => $name:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $kind {
            $( $(#[$variant_meta])* $variant, )+
        }

        impl $kind {
            /// Every value, in declaration order.
            pub const ALL: &'static [Self] = &[ $( Self::$variant, )+ ];

            /// The stored name.
            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $( Self::$variant => $name, )+
                }
            }

            /// The value with this stored name.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|value| value.name() == name)
            }
        }
    };
}

named_kind! {
    /// What a symbol is.
    SymbolKind {
        /// A function outside any type, including an SQL function or procedure.
        Function => "function",
        /// A function that belongs to a type.
        Method => "method",
        /// A class, struct, record or enum.
        Class => "class",
        /// An interface, trait or protocol.
        Interface => "interface",
        /// A type alias or another named type, including an SQL table or view.
        Type => "type",
        /// A constant or static, or a top-level key or table in a data file.
        Constant => "constant",
        /// A module, namespace or package, or a heading in a document.
        Module => "module",
        /// A test function.
        Test => "test",
    }
}

named_kind! {
    /// How a reference uses its target.
    RefKind {
        /// A call.
        Call => "call",
        /// An import of a module or a name.
        Import => "import",
        /// Inheritance or implementation.
        Inherit => "inherit",
        /// A use of a type.
        TypeRef => "type_ref",
        /// A test of the target.
        TestOf => "test_of",
    }
}

named_kind! {
    /// How a reference was tied to its target.
    Resolution {
        /// A precise index the user or CI produced.
        Scip => "scip",
        /// The import path resolved to a file that exports the name.
        ImportPath => "import_path",
        /// The only definition with the name in the same file.
        SameFile => "same_file",
        /// The only definition with the name among the files of the reference's language family.
        NameOnly => "name_only",
    }
}

impl Resolution {
    /// How far a reference resolved this way can be trusted, from 0 (not at all) to 1 (certain).
    #[must_use]
    pub fn confidence(self) -> f64 {
        match self {
            Self::Scip => 1.0,
            Self::ImportPath => 0.8,
            Self::SameFile => 0.6,
            Self::NameOnly => 0.3,
        }
    }
}

named_kind! {
    /// What a graph node stands for.
    NodeKind {
        /// A folder.
        Dir => "dir",
        /// A file.
        File => "file",
        /// A symbol.
        Symbol => "symbol",
        /// A project fact.
        Fact => "fact",
        /// A recorded task.
        Episode => "episode",
    }
}

/// The type of a graph edge, stored as a small number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeType {
    /// From code structure: imports, calls, references, inheritance, tests.
    Structural,
    /// From files that change together in git history.
    CoChange,
    /// From code used together in successful tasks.
    CoAccess,
    /// From a fact to the code it is about.
    About,
    /// From a folder to what it holds.
    Containment,
}

impl EdgeType {
    /// Every type, in stored-number order.
    pub const ALL: &'static [Self] = &[
        Self::Structural,
        Self::CoChange,
        Self::CoAccess,
        Self::About,
        Self::Containment,
    ];

    /// The stored number.
    #[must_use]
    pub fn number(self) -> u8 {
        match self {
            Self::Structural => 0,
            Self::CoChange => 1,
            Self::CoAccess => 2,
            Self::About => 3,
            Self::Containment => 4,
        }
    }

    /// The type with this stored number.
    #[must_use]
    pub fn from_number(number: u8) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.number() == number)
    }
}

/// Facts about a file, stored as bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileFlags(u32);

impl FileFlags {
    /// Produced by a tool: indexed at file level only, never shown as code.
    pub const GENERATED: Self = Self(1);
    /// Third-party code kept in the repository: file level only.
    pub const VENDORED: Self = Self(2);
    /// Not text: a NUL byte near its start.
    pub const BINARY: Self = Self(4);
    /// Larger than the configured limit: recorded, never read.
    pub const TOO_LARGE: Self = Self(8);
    /// Symbols and references have been extracted.
    pub const PARSED: Self = Self(16);
    /// Its name matches a secret pattern: recorded, never opened.
    pub const SECRET_PATTERN: Self = Self(32);

    /// No flags.
    #[must_use]
    pub fn empty() -> Self {
        Self(0)
    }

    /// The stored bits.
    #[must_use]
    pub fn bits(self) -> u32 {
        self.0
    }

    /// Flags read back from storage.
    #[must_use]
    pub fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// Whether every flag in `other` is set.
    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// These flags with `other` added.
    #[must_use]
    pub fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every stored name reads back to its value, and the lists are the ones storage documents.
    #[test]
    fn names_read_back() {
        let symbols: Vec<&str> = SymbolKind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(
            symbols,
            [
                "function",
                "method",
                "class",
                "interface",
                "type",
                "constant",
                "module",
                "test"
            ]
        );
        let refs: Vec<&str> = RefKind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(refs, ["call", "import", "inherit", "type_ref", "test_of"]);
        let nodes: Vec<&str> = NodeKind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(nodes, ["dir", "file", "symbol", "fact", "episode"]);
        for kind in SymbolKind::ALL {
            assert_eq!(SymbolKind::from_name(kind.name()), Some(*kind));
        }
        for kind in Resolution::ALL {
            assert_eq!(Resolution::from_name(kind.name()), Some(*kind));
        }
        assert_eq!(RefKind::from_name("calls"), None);
    }

    /// Resolution confidences and edge numbers as storage documents them.
    #[test]
    fn numbers_are_fixed() {
        let confidences: Vec<(&str, f64)> = Resolution::ALL
            .iter()
            .map(|kind| (kind.name(), kind.confidence()))
            .collect();
        assert_eq!(
            confidences,
            [
                ("scip", 1.0),
                ("import_path", 0.8),
                ("same_file", 0.6),
                ("name_only", 0.3)
            ]
        );
        let numbers: Vec<u8> = EdgeType::ALL.iter().map(|kind| kind.number()).collect();
        assert_eq!(numbers, [0, 1, 2, 3, 4]);
        assert_eq!(EdgeType::from_number(4), Some(EdgeType::Containment));
        assert_eq!(EdgeType::from_number(5), None);
    }

    /// Flag bits combine and test as documented: generated 1, vendored 2, binary 4, too large 8,
    /// parsed 16, secret pattern 32.
    #[test]
    fn flags_combine() {
        let bits: Vec<u32> = [
            FileFlags::GENERATED,
            FileFlags::VENDORED,
            FileFlags::BINARY,
            FileFlags::TOO_LARGE,
            FileFlags::PARSED,
            FileFlags::SECRET_PATTERN,
        ]
        .iter()
        .map(|flag| flag.bits())
        .collect();
        assert_eq!(bits, [1, 2, 4, 8, 16, 32]);
        let flags = FileFlags::empty()
            .with(FileFlags::GENERATED)
            .with(FileFlags::PARSED);
        assert_eq!(flags.bits(), 17);
        assert!(flags.contains(FileFlags::PARSED));
        assert!(!flags.contains(FileFlags::BINARY));
        // One of the two flags asked about is missing, so the pair is not contained.
        assert!(!flags.contains(FileFlags::GENERATED.with(FileFlags::BINARY)));
        // Adding a flag that is already set changes nothing.
        assert_eq!(flags.with(FileFlags::PARSED), flags);
        assert_eq!(FileFlags::from_bits(17), flags);
    }
}
