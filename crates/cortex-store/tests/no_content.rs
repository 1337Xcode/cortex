//! What the store adds on its own: after a file's records are written, no line of the file's
//! source appears anywhere in the database file. The records here are built by hand from the
//! source; the same scan over records an indexer builds belongs with the indexer's tests.

mod common;

use common::{Scratch, TestResult, name, path};
use cortex_core::{ContentHash, FileFlags, Language, RefKind, ShortHash, SymbolKind};
use cortex_store::{FileIndex, FileRecord, RefRecord, SymbolRecord};

/// A small Python file. Its definitions and uses are recorded below by hand, with byte offsets
/// counted from this text: line 3 starts at byte 11, line 4 at 25, line 5 at 52, line 7 at 89 and
/// line 8 at 108; the text is 133 bytes long.
const SOURCE: &str = "import os\n\nclass Parser:\n    def parse(self, text):\n        return helper(text.strip())\n\ndef helper(value):\n    return value.upper()\n";

/// Lines shorter than this are too common to count as code, such as `import os`.
const MIN_LINE_CHARS: usize = 12;

/// A definition spanning `start..end` of [`SOURCE`] whose header ends at `header_end`.
fn definition(
    kind: SymbolKind,
    (name_text, qual_name): (&str, &str),
    parent: Option<usize>,
    (start, header_end, end): (usize, usize, usize),
    lines: (u32, u32),
) -> TestResult<SymbolRecord> {
    let bytes = SOURCE.as_bytes();
    let header = bytes
        .get(start..header_end)
        .ok_or("header outside the source")?;
    let whole = bytes
        .get(start..end)
        .ok_or("definition outside the source")?;
    Ok(SymbolRecord {
        parent,
        kind,
        name: name(name_text)?,
        qual_name: name(qual_name)?,
        start_byte: u64::try_from(start)?,
        end_byte: u64::try_from(end)?,
        start_line: lines.0,
        end_line: lines.1,
        sig_hash: ShortHash::of(header),
        body_hash: ShortHash::of(whole),
    })
}

/// A use of `dst_name` on `line`, inside the symbol at `symbol`.
fn use_of(
    symbol: Option<usize>,
    dst_name: &str,
    kind: RefKind,
    line: u32,
) -> TestResult<RefRecord> {
    Ok(RefRecord {
        symbol,
        dst_name: name(dst_name)?,
        kind,
        line,
    })
}

/// Everything the index would record about [`SOURCE`].
fn parser_index() -> TestResult<FileIndex> {
    Ok(FileIndex {
        file: FileRecord {
            path: path("src/parser.py")?,
            language: Some(Language::Python),
            size_bytes: 133,
            line_count: Some(8),
            content_hash: Some(ContentHash::of(SOURCE.as_bytes())),
            mtime_ns: 1,
            flags: FileFlags::empty().with(FileFlags::PARSED),
            indexed_at_ms: 1,
        },
        symbols: vec![
            definition(
                SymbolKind::Class,
                ("Parser", "Parser"),
                None,
                (11, 24, 87),
                (3, 5),
            )?,
            definition(
                SymbolKind::Method,
                ("parse", "Parser.parse"),
                Some(0),
                (29, 51, 87),
                (4, 5),
            )?,
            definition(
                SymbolKind::Function,
                ("helper", "helper"),
                None,
                (89, 107, 132),
                (7, 8),
            )?,
        ],
        refs: vec![
            use_of(None, "os", RefKind::Import, 1)?,
            use_of(Some(1), "helper", RefKind::Call, 5)?,
            use_of(Some(1), "strip", RefKind::Call, 5)?,
            use_of(Some(2), "upper", RefKind::Call, 8)?,
        ],
        passes_on: Vec::new(),
    })
}

/// Whether `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The offsets used above match the source, so the records describe it truthfully.
#[test]
fn the_offsets_match_the_source() {
    assert_eq!(SOURCE.len(), 133);
    assert_eq!(&SOURCE[11..24], "class Parser:");
    assert_eq!(&SOURCE[29..51], "def parse(self, text):");
    assert_eq!(&SOURCE[89..107], "def helper(value):");
    assert!(SOURCE[..132].ends_with("value.upper()"));
}

/// Once a file's records are written and the writer has closed, no source line of
/// [`MIN_LINE_CHARS`] or more appears in the database file or its log.
#[test]
fn no_source_line_reaches_the_database() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&parser_index()?)?;
    transaction.commit()?;
    drop(writer);
    let mut stored = std::fs::read(scratch.folder.join("cortex.db"))?;
    let log = scratch.folder.join("cortex.db-wal");
    if log.try_exists()? {
        stored.extend(std::fs::read(log)?);
    }
    // The names were stored, so the scan reads the data it is meant to check.
    assert!(contains(&stored, b"Parser.parse"));
    let lines: Vec<&str> = SOURCE
        .lines()
        .map(str::trim)
        .filter(|line| line.chars().count() >= MIN_LINE_CHARS)
        .collect();
    assert_eq!(lines.len(), 5);
    for line in lines {
        assert!(!contains(&stored, line.as_bytes()), "stored: {line}");
    }
    Ok(())
}
