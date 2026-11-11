//! Change cards: what Cortex keeps of a commit that owns lines of the code it shows. A card holds
//! the commit's id and first parent, when it was committed, what kind of change it is, the pull
//! request and issues it names, whether its message gives a reason, whether it touched tests, and
//! the files and symbols it changed as node ids. No subject, message or person's name is kept: the
//! daemon holds the text in memory and reads it from git again when it is needed.
//!
//! A card's terms, the words a prompt may share with the change, are kept apart in a full-text
//! index and passed in by the caller, which is the only one that sees the message. Cards are
//! small, so nothing here prunes them; `WriteTx::remove_change_cards` removes
//! chosen ones with their terms.

use cortex_core::is_object_id;
use rusqlite::{Connection, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::schema;
use crate::search::match_expression;
use crate::values;

/// The most issue references a card keeps.
pub const MAX_CARD_ISSUES: usize = 3;

/// The most node ids a card keeps in each of its lists of files, symbols and test files, so that a
/// row stays small. A card is refused when a list is longer; the caller stores none for a mass
/// change, which touches too many files to say anything about one.
pub const MAX_CARD_NODES: usize = 256;

/// The most characters an issue reference has: an owner, a repository and a number.
const MAX_ISSUE_CHARS: usize = 160;

/// What kind of change a commit is, which decides whether its message is worth showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// A change whose message may give a reason.
    Normal,
    /// A change whose subject names housekeeping, or that only moved whitespace.
    Trivial,
    /// A change to more paths than a change that is about one thing touches.
    Mass,
    /// A change made by a bot.
    Bot,
}

impl CardKind {
    /// The name stored for the kind.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Trivial => "trivial",
            Self::Mass => "mass",
            Self::Bot => "bot",
        }
    }

    /// The kind stored under `name`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Normal, Self::Trivial, Self::Mass, Self::Bot]
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

/// A commit as the store keeps it, written and read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeCardRecord {
    /// The commit's id, as lower-case hexadecimal.
    pub oid: String,
    /// The id of the commit's first parent; none for a root commit.
    pub first_parent: Option<String>,
    /// When the commit was committed, in UTC seconds.
    pub committed_at_s: i64,
    /// The kind of change.
    pub kind: CardKind,
    /// The pull or merge request the commit came from.
    pub pull_request: Option<u64>,
    /// The issues the message names, as written (`#12`, `owner/repo#12`), at most
    /// [`MAX_CARD_ISSUES`].
    pub issues: Vec<String>,
    /// Whether the message has a line that gives a reason.
    pub has_reason: bool,
    /// The nodes of the files the commit changed, at most [`MAX_CARD_NODES`].
    pub files: Vec<NodeId>,
    /// The nodes of the symbols the commit changed, at most [`MAX_CARD_NODES`].
    pub symbols: Vec<NodeId>,
    /// The nodes of the test files the commit changed, at most [`MAX_CARD_NODES`].
    pub tests: Vec<NodeId>,
    /// When the card was made, in UTC milliseconds.
    pub carded_at_ms: i64,
}

impl Queries<'_> {
    /// The cards of the commits in `oids`, by commit; a commit with no card is left out.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored cards that cannot be read back.
    pub fn change_cards(self, oids: &[String]) -> Result<Vec<ChangeCardRecord>, StoreError> {
        let listed = list_of(oids)?;
        self.read_cards(schema::CARDS_BY_OID, (listed,))
    }

    /// The cards of the commits that came from pull request `number`, or whose message names the
    /// issue `#number` of this repository, newest first and then by commit. An issue of another
    /// repository (`owner/repo#12`) does not answer.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored cards that cannot be read back.
    pub fn change_cards_numbered(self, number: u64) -> Result<Vec<ChangeCardRecord>, StoreError> {
        let number = values::integer(number, "change_cards.pr")?;
        self.read_cards(schema::CARDS_NUMBERED, (number, format!("#{number}")))
    }

    /// The `limit` cards whose terms best match any of `words`, best first: each with its BM25
    /// score, the higher the better. Cards that score alike come in commit order.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored cards that cannot be read back.
    pub fn search_change_cards(
        self,
        words: &[&str],
        limit: usize,
    ) -> Result<Vec<(ChangeCardRecord, f64)>, StoreError> {
        if words.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows: Vec<(RawCard, f64)> = self
            .connection()
            .prepare_cached(schema::CARDS_MATCHING)
            .and_then(|mut statement| {
                statement
                    .query_map((match_expression(words), limit), |row| {
                        Ok((RawCard::read(row)?, row.get(11)?))
                    })?
                    .collect()
            })
            .map_err(sqlite("matching words to past changes"))?;
        rows.into_iter()
            .map(|(raw, score)| Ok((raw.check()?, score)))
            .collect()
    }

    /// How many cards are kept.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn change_card_count(self) -> Result<u64, StoreError> {
        let count: i64 = self
            .connection()
            .query_row(schema::CARD_ROWS, [], |row| row.get(0))
            .map_err(sqlite("counting change cards"))?;
        values::number(count, "change_cards.count", count)
    }

    /// The cards `sql` reads with `params`.
    fn read_cards(
        self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<ChangeCardRecord>, StoreError> {
        let rows: Vec<RawCard> = self
            .connection()
            .prepare_cached(sql)
            .and_then(|mut statement| statement.query_map(params, RawCard::read)?.collect())
            .map_err(sqlite("reading change cards"))?;
        rows.into_iter().map(RawCard::check).collect()
    }
}

/// Stores `card` and its `tokens`, replacing the card of the same commit and its terms.
pub(crate) fn put(
    connection: &Connection,
    card: &ChangeCardRecord,
    tokens: &[String],
) -> Result<(), StoreError> {
    check_card(card)?;
    let existed = connection
        .prepare_cached(schema::CARD_EXISTS)
        .and_then(|mut statement| statement.exists([&card.oid]))
        .map_err(sqlite("looking for a change card"))?;
    insert_card(connection, card)?;
    // A card seen before has terms to replace; a new card has none, and finding them would read
    // the whole index.
    if existed {
        delete_terms(connection, std::slice::from_ref(&card.oid))?;
    }
    // INVARIANT: no-content-stored. The terms are the words the caller chose; the store is never
    // given the subject or the message they came from.
    if tokens.is_empty() {
        return Ok(());
    }
    connection
        .prepare_cached(schema::TERMS_INSERT)
        .and_then(|mut statement| statement.execute((&card.oid, tokens.join(" "))))
        .map(drop)
        .map_err(sqlite("writing the terms of a change card"))
}

/// Writes the row of `card`, replacing the row of the same commit.
fn insert_card(connection: &Connection, card: &ChangeCardRecord) -> Result<(), StoreError> {
    let pull_request = card
        .pull_request
        .map(|number| values::integer(number, "change_cards.pr"))
        .transpose()?;
    let issues = json(&card.issues, "change_cards.issues")?;
    let files = json(&node_ids(&card.files), "change_cards.file_nodes")?;
    let symbols = json(&node_ids(&card.symbols), "change_cards.symbol_nodes")?;
    let tests = json(&node_ids(&card.tests), "change_cards.test_nodes")?;
    connection
        .prepare_cached(schema::CARD_PUT)
        .and_then(|mut statement| {
            statement.execute(rusqlite::params![
                card.oid,
                card.first_parent,
                card.committed_at_s,
                card.kind.name(),
                pull_request,
                issues,
                card.has_reason,
                files,
                symbols,
                tests,
                card.carded_at_ms,
            ])
        })
        .map(drop)
        .map_err(sqlite("writing a change card"))
}

/// The ids of `nodes`, as stored.
fn node_ids(nodes: &[NodeId]) -> Vec<i64> {
    nodes.iter().map(|node| node.get()).collect()
}

/// `value` as the JSON text of `column`.
fn json<T: serde::Serialize + ?Sized>(
    value: &T,
    column: &'static str,
) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::Invalid { column })
}

/// Removes the cards of the commits in `oids`, with their terms: how many cards.
pub(crate) fn remove(connection: &Connection, oids: &[String]) -> Result<u64, StoreError> {
    if oids.is_empty() {
        return Ok(0);
    }
    delete_terms(connection, oids)?;
    let listed = list_of(oids)?;
    let removed = connection
        .prepare_cached(schema::CARDS_DELETE)
        .and_then(|mut statement| statement.execute([listed]))
        .map_err(sqlite("removing change cards"))?;
    Ok(u64::try_from(removed).unwrap_or(u64::MAX))
}

/// Deletes the terms of the commits in `oids`.
fn delete_terms(connection: &Connection, oids: &[String]) -> Result<(), StoreError> {
    let listed = list_of(oids)?;
    connection
        .prepare_cached(schema::TERMS_DELETE)
        .and_then(|mut statement| statement.execute([listed]))
        .map(drop)
        .map_err(sqlite("removing the terms of change cards"))
}

/// `oids` as a JSON array of strings, which SQLite reads back with `json_each`.
fn list_of(oids: &[String]) -> Result<String, StoreError> {
    serde_json::to_string(oids).map_err(|_| StoreError::Invalid {
        column: "change_cards.oid",
    })
}

/// Refuses a card that holds anything but ids, numbers and references.
fn check_card(card: &ChangeCardRecord) -> Result<(), StoreError> {
    let invalid = |column| Err(StoreError::Invalid { column });
    if !is_object_id(&card.oid) {
        return invalid("change_cards.oid");
    }
    if card
        .first_parent
        .as_deref()
        .is_some_and(|parent| !is_object_id(parent))
    {
        return invalid("change_cards.first_parent");
    }
    if card.issues.len() > MAX_CARD_ISSUES || !card.issues.iter().all(|issue| is_reference(issue)) {
        return invalid("change_cards.issues");
    }
    let lists = [
        (&card.files, "change_cards.file_nodes"),
        (&card.symbols, "change_cards.symbol_nodes"),
        (&card.tests, "change_cards.test_nodes"),
    ];
    for (nodes, column) in lists {
        if nodes.len() > MAX_CARD_NODES {
            return invalid(column);
        }
    }
    Ok(())
}

/// Whether `text` could be an issue reference: `#12` or `owner/repo#12`, in the characters such
/// a name and number have, and not a sentence.
fn is_reference(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ISSUE_CHARS
        && text.contains('#')
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'#' | b'/' | b'.' | b'_' | b'-')
        })
}

/// A card as its columns hold it, before it is checked.
struct RawCard {
    /// `oid`.
    oid: String,
    /// `first_parent`.
    first_parent: Option<String>,
    /// `committed_at`.
    committed_at: i64,
    /// `kind`.
    kind: String,
    /// `pr`.
    pr: Option<i64>,
    /// `issues`.
    issues: String,
    /// `has_reason`.
    has_reason: i64,
    /// `file_nodes`.
    file_nodes: String,
    /// `symbol_nodes`.
    symbol_nodes: String,
    /// `test_nodes`.
    test_nodes: String,
    /// `carded_at`.
    carded_at: i64,
}

impl RawCard {
    /// The card in the first eleven columns of `row`.
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            oid: row.get(0)?,
            first_parent: row.get(1)?,
            committed_at: row.get(2)?,
            kind: row.get(3)?,
            pr: row.get(4)?,
            issues: row.get(5)?,
            has_reason: row.get(6)?,
            file_nodes: row.get(7)?,
            symbol_nodes: row.get(8)?,
            test_nodes: row.get(9)?,
            carded_at: row.get(10)?,
        })
    }

    /// The card, when every column reads back as what it holds.
    fn check(self) -> Result<ChangeCardRecord, StoreError> {
        let corrupt = |column| StoreError::Corrupt {
            column,
            row: self.committed_at,
        };
        let flag = |value: i64, column| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(corrupt(column)),
        };
        let nodes = |text: &str, column| {
            serde_json::from_str::<Vec<i64>>(text)
                .map(|ids| ids.into_iter().map(NodeId).collect())
                .map_err(|_| corrupt(column))
        };
        Ok(ChangeCardRecord {
            first_parent: self.first_parent,
            committed_at_s: self.committed_at,
            kind: CardKind::from_name(&self.kind).ok_or(corrupt("change_cards.kind"))?,
            pull_request: self
                .pr
                .map(|number| values::number(number, "change_cards.pr", self.committed_at))
                .transpose()?,
            issues: serde_json::from_str(&self.issues)
                .map_err(|_| corrupt("change_cards.issues"))?,
            has_reason: flag(self.has_reason, "change_cards.has_reason")?,
            files: nodes(&self.file_nodes, "change_cards.file_nodes")?,
            symbols: nodes(&self.symbol_nodes, "change_cards.symbol_nodes")?,
            tests: nodes(&self.test_nodes, "change_cards.test_nodes")?,
            carded_at_ms: self.carded_at,
            oid: self.oid,
        })
    }
}
