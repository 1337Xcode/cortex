//! The history tables: cached blames read back by overlap, change cards read by commit, by pull
//! request or issue number and by BM25 over their terms, the refusals that keep text out, and the
//! upkeep that drops stale and surplus blames. Expected values are worked by hand in each test.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{ContentHash, RepoPath};
use cortex_store::{
    BlameRecord, BlameRunRecord, CardKind, ChangeCardRecord, MAX_CARD_NODES, StoreError, Writer,
};

/// A commit id of 40 copies of `digit`.
fn oid(digit: char) -> String {
    digit.to_string().repeat(40)
}

/// The hash of the content `text`.
fn hash(text: &str) -> ContentHash {
    ContentHash::of(text.as_bytes())
}

/// A blame of lines `first` to `last` of the file at `path` whose content hashed `file_hash`,
/// computed at `computed_at_ms`, with one run over all of them from the commit `digit`.
fn blame(
    (path, file_hash): (&RepoPath, ContentHash),
    (first, last): (u32, u32),
    (digit, computed_at_ms): (char, i64),
) -> BlameRecord {
    BlameRecord {
        path: path.clone(),
        file_hash,
        first_line: first,
        last_line: last,
        runs: vec![BlameRunRecord {
            start: first,
            end: last,
            source_start: first + 100,
            oid: oid(digit),
        }],
        computed_at_ms,
    }
}

/// A card of the commit `digit` with nothing set but what a card needs.
fn card(digit: char, kind: CardKind) -> ChangeCardRecord {
    ChangeCardRecord {
        oid: oid(digit),
        first_parent: None,
        committed_at_s: 1_000,
        kind,
        pull_request: None,
        issues: Vec::new(),
        has_reason: false,
        tests: Vec::new(),
        files: Vec::new(),
        symbols: Vec::new(),
        carded_at_ms: 9_000,
    }
}

/// Words as the owned list a caller passes.
fn words(text: &str) -> Vec<String> {
    text.split(' ').map(str::to_owned).collect()
}

/// The first lines of `rows`, which name the blames in a result.
fn first_lines(rows: &[BlameRecord]) -> Vec<u32> {
    rows.iter().map(|row| row.first_line).collect()
}

/// The commit ids of `cards`, as the first digit of each.
fn digits(cards: &[ChangeCardRecord]) -> String {
    cards
        .iter()
        .filter_map(|card| card.oid.chars().next())
        .collect()
}

/// Stores `blames` in one transaction.
fn store_blames(writer: &mut Writer, blames: &[BlameRecord]) -> TestResult {
    let mut transaction = writer.transaction()?;
    for record in blames {
        transaction.put_blame(record)?;
    }
    transaction.commit()?;
    Ok(())
}

/// Blames A (lines 1 to 10), B (5 to 20) and C (30 to 40) of one content, and D (1 to 100) of
/// another. A range overlaps a blame when it starts at or before the blame's last line and ends at
/// or after its first, the edges counting: 10 to 12 meets A (at its last line) and B; 11 to 29
/// meets B alone; 20 to 30 meets B at its last line and C at its first; 21 to 29 and 41 to 50 meet
/// none; 1 to 1 meets A; 8 to 32 meets all three, which the caller combines. D is of another
/// content, which no range of the first reaches.
#[test]
fn cached_blames_come_back_where_their_lines_overlap() -> TestResult {
    let a = RepoPath::new("a.py")?;
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let (one, two) = (hash("one"), hash("two"));
    store_blames(
        &mut writer,
        &[
            blame((&a, one), (1, 10), ('a', 100)),
            blame((&a, one), (5, 20), ('b', 200)),
            blame((&a, one), (30, 40), ('c', 300)),
            blame((&a, two), (1, 100), ('d', 400)),
        ],
    )?;
    let queries = writer.queries();
    let cases: [(u32, u32, &[u32]); 7] = [
        (10, 12, &[1, 5]),
        (11, 29, &[5]),
        (20, 30, &[5, 30]),
        (21, 29, &[]),
        (41, 50, &[]),
        (1, 1, &[1]),
        (8, 32, &[1, 5, 30]),
    ];
    for (first, last, expected) in cases {
        let found = queries.cached_blames((&a, &one), first, last)?;
        assert_eq!(first_lines(&found), expected, "{first} to {last}");
    }
    assert_eq!(
        first_lines(&queries.cached_blames((&a, &two), 50, 60)?),
        [1]
    );
    assert_eq!(queries.cached_blames((&a, &hash("three")), 1, 100)?, []);
    // What was written is what comes back: the runs, the range, the time and the content.
    assert_eq!(
        queries.cached_blames((&a, &one), 30, 30)?,
        [blame((&a, one), (30, 40), ('c', 300))]
    );
    assert_eq!(queries.cached_blame_count()?, 4);
    Ok(())
}

/// Blaming the same range of the same content again replaces the first blame: the count stays and
/// the runs and the time are the new ones, with two runs read back in the order they were given.
#[test]
fn a_blame_of_the_same_range_is_replaced() -> TestResult {
    let a = RepoPath::new("a.py")?;
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let file_hash = hash("one");
    let mut newer = blame((&a, file_hash), (1, 10), ('b', 500));
    newer.runs = vec![
        BlameRunRecord {
            start: 1,
            end: 4,
            source_start: 7,
            oid: oid('b'),
        },
        BlameRunRecord {
            start: 5,
            end: 10,
            source_start: 1,
            oid: oid('c'),
        },
    ];
    store_blames(
        &mut writer,
        &[blame((&a, file_hash), (1, 10), ('a', 100)), newer.clone()],
    )?;
    assert_eq!(writer.queries().cached_blame_count()?, 1);
    assert_eq!(
        writer.queries().cached_blames((&a, &file_hash), 1, 10)?,
        [newer]
    );
    Ok(())
}

/// A blame that holds anything but line numbers in order and commit ids is refused, and a refused
/// write leaves nothing behind.
#[test]
fn a_blame_that_is_not_lines_and_commit_ids_is_refused() -> TestResult {
    let a = RepoPath::new("a.py")?;
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let file_hash = hash("one");
    let mut not_an_id = blame((&a, file_hash), (1, 10), ('a', 1));
    not_an_id.runs[0].oid = "fix the parser, it dropped tokens".to_owned();
    let refused = [
        not_an_id,
        blame((&a, file_hash), (0, 10), ('a', 1)),
        blame((&a, file_hash), (11, 10), ('a', 1)),
    ];
    for record in &refused {
        let mut transaction = writer.transaction()?;
        let error = transaction.put_blame(record).err();
        assert!(
            matches!(error, Some(StoreError::Invalid { .. })),
            "{record:?}: {error:?}"
        );
    }
    assert_eq!(writer.queries().cached_blame_count()?, 0);
    Ok(())
}

/// Cards read by commit come in commit order and leave out a commit with no card; every field
/// comes back as written, with the node ids of real files.
#[test]
fn cards_are_read_back_as_written_by_commit() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let first = transaction
        .replace_file(&index("a.py", Vec::new(), Vec::new())?)?
        .file_node;
    let second = transaction
        .replace_file(&index("b.py", Vec::new(), Vec::new())?)?
        .file_node;
    let tested = transaction
        .replace_file(&index("test_b.py", Vec::new(), Vec::new())?)?
        .file_node;
    let full = ChangeCardRecord {
        first_parent: Some(oid('f')),
        committed_at_s: 3_000,
        pull_request: Some(12),
        issues: vec!["#7".to_owned(), "owner/repo#9".to_owned()],
        has_reason: true,
        tests: vec![tested],
        files: vec![first, second],
        symbols: vec![second],
        ..card('a', CardKind::Normal)
    };
    transaction.put_change_card(&full, &words("alpha"))?;
    transaction.put_change_card(&card('d', CardKind::Bot), &[])?;
    transaction.commit()?;
    let queries = writer.queries();
    let found = queries.change_cards(&[oid('d'), oid('a'), oid('e')])?;
    assert_eq!(found, [full, card('d', CardKind::Bot)]);
    assert_eq!(queries.change_cards(&[])?, []);
    assert_eq!(queries.change_card_count()?, 2);
    Ok(())
}

/// Stores the cards of `numbered_cards`, a worked set. Commit a came from pull request 12 and
/// names issue #7 (3,000); b names issue #12 and another repository's #7 (2,000); c came from
/// pull request 7 and names issue #7 (1,000); d names nothing.
fn store_numbered_cards(writer: &mut Writer) -> TestResult {
    let mut transaction = writer.transaction()?;
    let cards = [
        ('a', Some(12), vec!["#7"], 3_000),
        ('b', None, vec!["#12", "owner/repo#7"], 2_000),
        ('c', Some(7), vec!["#7"], 1_000),
        ('d', None, vec![], 500),
    ];
    for (digit, pull_request, issues, committed_at_s) in cards {
        let record = ChangeCardRecord {
            pull_request,
            issues: issues.into_iter().map(str::to_owned).collect(),
            committed_at_s,
            ..card(digit, CardKind::Normal)
        };
        transaction.put_change_card(&record, &[])?;
    }
    transaction.commit()?;
    Ok(())
}

/// Number 12 is the pull request of a and the issue of b, newest first; number 7 is the issue of
/// a and of c, which is also its pull request, so c comes once; b's `owner/repo#7` is another
/// repository's issue and does not answer; no card has number 99.
#[test]
fn cards_are_found_by_pull_request_and_issue_number() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    store_numbered_cards(&mut writer)?;
    let queries = writer.queries();
    assert_eq!(digits(&queries.change_cards_numbered(12)?), "ab");
    assert_eq!(digits(&queries.change_cards_numbered(7)?), "ac");
    assert_eq!(digits(&queries.change_cards_numbered(99)?), "");
    Ok(())
}

/// Eight cards: a, b and c hold the word `retry` and five fillers do not. Against the word
/// `retry` BM25 favours a word said more often and a shorter text, and the three are alike in
/// everything else: a says it three times in four words, b once in two, c once in six, so the
/// order is a, b, c, each score lower than the last and above 0. The fillers match nothing.
fn store_bm25_cards(writer: &mut Writer) -> TestResult {
    let mut transaction = writer.transaction()?;
    let texts = [
        ('a', "retry retry retry backoff"),
        ('b', "retry backoff"),
        ('c', "retry backoff jitter plan more words"),
        ('1', "unrelated words"),
        ('2', "unrelated words"),
        ('3', "unrelated words"),
        ('4', "unrelated words"),
        ('5', "unrelated words"),
    ];
    for (digit, text) in texts {
        transaction.put_change_card(&card(digit, CardKind::Normal), &words(text))?;
    }
    transaction.commit()?;
    Ok(())
}

/// The best cards for a word come in BM25 order with their scores, the limit cuts the list, a word
/// no card holds finds none, and cards that score alike (the fillers) come in commit order.
#[test]
fn cards_are_ranked_by_bm25_over_their_terms() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    store_bm25_cards(&mut writer)?;
    let queries = writer.queries();
    let found = queries.search_change_cards(&["retry", "missing"], 10)?;
    let order: String = found
        .iter()
        .filter_map(|(card, _)| card.oid.chars().next())
        .collect();
    assert_eq!(order, "abc");
    let scores: Vec<f64> = found.iter().map(|(_, score)| *score).collect();
    assert!(scores.iter().all(|score| *score > 0.0), "{scores:?}");
    assert!(
        scores.windows(2).all(|pair| pair[0] > pair[1]),
        "{scores:?}"
    );
    assert_eq!(queries.search_change_cards(&["retry"], 2)?.len(), 2);
    assert_eq!(queries.search_change_cards(&["nothing"], 10)?, []);
    assert_eq!(queries.search_change_cards(&[], 10)?, []);
    assert_eq!(queries.search_change_cards(&["retry"], 0)?, []);
    let tied = queries.search_change_cards(&["unrelated"], 3)?;
    let tied: String = tied
        .iter()
        .filter_map(|(card, _)| card.oid.chars().next())
        .collect();
    assert_eq!(tied, "123");
    // A word with a quote or the operator `OR` in it is a word, never part of the question.
    assert_eq!(queries.search_change_cards(&["a\" OR \"retry"], 10)?, []);
    Ok(())
}

/// Putting a card again replaces its terms (a now holds only `other`, so `retry` finds b and c),
/// putting it with no terms leaves none, and removing cards takes their terms: with a and b
/// removed, a commit with no card counted in the removal's result as nothing, `retry` finds c.
#[test]
fn terms_follow_their_card_when_it_is_replaced_and_removed() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    store_bm25_cards(&mut writer)?;
    let mut transaction = writer.transaction()?;
    transaction.put_change_card(&card('a', CardKind::Normal), &words("other"))?;
    transaction.commit()?;
    let found = |writer: &Writer, word: &str| -> TestResult<String> {
        Ok(writer
            .queries()
            .search_change_cards(&[word], 10)?
            .iter()
            .filter_map(|(card, _)| card.oid.chars().next())
            .collect())
    };
    assert_eq!(found(&writer, "retry")?, "bc");
    assert_eq!(found(&writer, "other")?, "a");
    let mut transaction = writer.transaction()?;
    transaction.put_change_card(&card('a', CardKind::Normal), &[])?;
    assert_eq!(transaction.remove_change_cards(&[oid('b'), oid('9')])?, 1);
    transaction.commit()?;
    assert_eq!(found(&writer, "other")?, "");
    assert_eq!(found(&writer, "retry")?, "c");
    assert_eq!(writer.queries().change_card_count()?, 7);
    assert_eq!(
        digits(&writer.queries().change_cards(&[oid('a'), oid('b')])?),
        "a"
    );
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.remove_change_cards(&[])?, 0);
    Ok(())
}

/// A card that holds anything but ids, numbers, a kind and references is refused: an id that is
/// not hexadecimal (a subject, an upper-case id), a parent that is not one, four references, a
/// reference that is a sentence or has no number sign. Three references of any repository are
/// kept.
#[test]
fn a_card_that_is_not_ids_numbers_and_references_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let with = |change: &dyn Fn(&mut ChangeCardRecord)| {
        let mut record = card('a', CardKind::Normal);
        change(&mut record);
        record
    };
    let refused = [
        with(&|card| card.oid = "fix the parser".to_owned()),
        with(&|card| card.oid = oid('A')),
        with(&|card| card.first_parent = Some("the parent".to_owned())),
        with(&|card| card.issues = words("#1 #2 #3 #4")),
        with(&|card| card.issues = vec!["fixes the crash #12".to_owned()]),
        with(&|card| card.issues = vec!["12".to_owned()]),
    ];
    for record in &refused {
        let mut transaction = writer.transaction()?;
        let error = transaction.put_change_card(record, &[]).err();
        assert!(
            matches!(error, Some(StoreError::Invalid { .. })),
            "{record:?}: {error:?}"
        );
    }
    assert_eq!(writer.queries().change_card_count()?, 0);
    let mut transaction = writer.transaction()?;
    let kept = with(&|card| card.issues = words("#1 owner/repo#2 other.org/a-b_c#3"));
    transaction.put_change_card(&kept, &[])?;
    transaction.commit()?;
    assert_eq!(writer.queries().change_cards(&[oid('a')])?, [kept]);
    Ok(())
}

/// Stores a file at `path` whose content hashed `file_hash`.
fn store_file(writer: &mut Writer, path: &str, file_hash: ContentHash) -> TestResult {
    let mut record = index(path, Vec::new(), Vec::new())?;
    record.file.content_hash = Some(file_hash);
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&record)?;
    transaction.commit()?;
    Ok(())
}

/// Blames of `a.py` as the content `one`, which it has, and as `two` and `three`, which it has
/// not, are stored. A pass of limit 1 drops one of the two and leaves the kept one; a pass of limit
/// 10 drops the other; the kept blame stays until its file is read again with other content, and
/// then goes.
#[test]
fn blames_of_content_no_file_has_are_dropped() -> TestResult {
    let a = RepoPath::new("a.py")?;
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let (one, two, three) = (hash("one"), hash("two"), hash("three"));
    store_file(&mut writer, "a.py", one)?;
    store_blames(
        &mut writer,
        &[
            blame((&a, one), (1, 10), ('a', 1)),
            blame((&a, two), (1, 10), ('b', 2)),
            blame((&a, three), (1, 10), ('c', 3)),
        ],
    )?;
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.delete_blames_of_missing_files(1)?, 1);
    transaction.commit()?;
    assert_eq!(writer.queries().cached_blame_count()?, 2);
    assert_eq!(writer.queries().cached_blames((&a, &one), 1, 10)?.len(), 1);
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.delete_blames_of_missing_files(10)?, 1);
    assert_eq!(transaction.delete_blames_of_missing_files(10)?, 0);
    transaction.commit()?;
    assert_eq!(writer.queries().cached_blame_count()?, 1);
    store_file(&mut writer, "a.py", two)?;
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.delete_blames_of_missing_files(10)?, 1);
    transaction.commit()?;
    assert_eq!(writer.queries().cached_blame_count()?, 0);
    Ok(())
}

/// Five blames computed at 10, 20, 30, 40 and 50. Capping at 3 drops the two computed first (10 and
/// 20); capping again drops none; capping at 1 with a limit of 1 drops the oldest left (30) and no
/// more, and a cap of 5 on the two left drops none.
#[test]
fn the_cache_is_capped_by_dropping_the_oldest_computed() -> TestResult {
    let a = RepoPath::new("a.py")?;
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let file_hash = hash("one");
    let blames: Vec<BlameRecord> = (1..=5_u32)
        .zip(['a', 'b', 'c', 'd', 'e'])
        .map(|(line, digit)| blame((&a, file_hash), (line, line), (digit, i64::from(line) * 10)))
        .collect();
    // Written newest first, so that the order they were written in is not the order dropped.
    let reversed: Vec<BlameRecord> = blames.iter().rev().cloned().collect();
    store_blames(&mut writer, &reversed)?;
    let computed = |writer: &Writer| -> TestResult<Vec<i64>> {
        Ok(writer
            .queries()
            .cached_blames((&a, &file_hash), 1, 5)?
            .iter()
            .map(|row| row.computed_at_ms)
            .collect())
    };
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.cap_blames(3, 100)?, 2);
    assert_eq!(transaction.cap_blames(3, 100)?, 0);
    transaction.commit()?;
    assert_eq!(computed(&writer)?, [30, 40, 50]);
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.cap_blames(1, 1)?, 1);
    transaction.commit()?;
    assert_eq!(computed(&writer)?, [40, 50]);
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.cap_blames(5, 100)?, 0);
    Ok(())
}

/// A card keeps at most 256 node ids in each of its lists of files, symbols and test files: lists
/// of exactly 256 are kept whole, and a list of 257 in any one of the three is refused, leaving no
/// card behind.
#[test]
fn a_card_with_too_many_nodes_in_a_list_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let node = transaction
        .replace_file(&index("a.py", Vec::new(), Vec::new())?)?
        .file_node;
    transaction.commit()?;
    assert_eq!(MAX_CARD_NODES, 256);
    let full = ChangeCardRecord {
        files: vec![node; 256],
        symbols: vec![node; 256],
        tests: vec![node; 256],
        ..card('a', CardKind::Normal)
    };
    let refused = [
        ChangeCardRecord {
            files: vec![node; 257],
            ..full.clone()
        },
        ChangeCardRecord {
            symbols: vec![node; 257],
            ..full.clone()
        },
        ChangeCardRecord {
            tests: vec![node; 257],
            ..full.clone()
        },
    ];
    for record in &refused {
        let mut transaction = writer.transaction()?;
        let error = transaction.put_change_card(record, &[]).err();
        assert!(
            matches!(error, Some(StoreError::Invalid { .. })),
            "{error:?}"
        );
    }
    assert_eq!(writer.queries().change_card_count()?, 0);
    let mut transaction = writer.transaction()?;
    transaction.put_change_card(&full, &[])?;
    transaction.commit()?;
    assert_eq!(writer.queries().change_cards(&[oid('a')])?, [full]);
    Ok(())
}

/// Two files of the same content are blamed apart: `a.py` and `b.py` both hash to `one`, and the
/// blame of each comes back for its own path only, with its own commit. When `b.py` changes,
/// upkeep drops its blame and keeps `a.py`'s, whose file still has the content; a file with the
/// content at another path keeps no blame alive.
#[test]
fn files_of_the_same_content_are_blamed_apart() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let (a, b) = (RepoPath::new("a.py")?, RepoPath::new("b.py")?);
    let one = hash("one");
    store_file(&mut writer, "a.py", one)?;
    store_file(&mut writer, "b.py", one)?;
    let of_a = blame((&a, one), (1, 10), ('a', 1));
    let of_b = blame((&b, one), (1, 10), ('b', 2));
    store_blames(&mut writer, &[of_a.clone(), of_b.clone()])?;
    assert_eq!(
        writer.queries().cached_blames((&a, &one), 1, 10)?,
        std::slice::from_ref(&of_a)
    );
    assert_eq!(writer.queries().cached_blames((&b, &one), 1, 10)?, [of_b]);
    store_file(&mut writer, "b.py", hash("two"))?;
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.delete_blames_of_missing_files(10)?, 1);
    transaction.commit()?;
    assert_eq!(writer.queries().cached_blames((&a, &one), 1, 10)?, [of_a]);
    assert_eq!(writer.queries().cached_blame_count()?, 1);
    Ok(())
}
