//! Facts as the store keeps them: refused when they can never be held (too long, no anchor), read
//! back with their anchors, current until superseded, found by the nodes they are anchored to,
//! their uses counted and their confirmation stamped when made active again, and found when an
//! anchor's code changed or a status has lasted.

mod common;

use common::{Scratch, TestResult, index, symbol};
use cortex_core::{FactKind, FactSource, FactStatus, MAX_FACT_CHARS, ShortHash, SymbolKind, Ulid};
use cortex_store::{FactAnchor, FactId, FactRecord, NodeId, StoreError, Writer};

/// Files `paths` stored, their nodes in the same order.
fn files(writer: &mut Writer, paths: &[&str]) -> TestResult<Vec<NodeId>> {
    let mut transaction = writer.transaction()?;
    let mut nodes = Vec::new();
    for path in paths {
        nodes.push(
            transaction
                .replace_file(&index(path, Vec::new(), Vec::new())?)?
                .file_node,
        );
    }
    transaction.commit()?;
    Ok(nodes)
}

/// A fact saying `text`, from `source`, made at `at_ms`, anchored to `nodes`, each with a hash
/// made from its id. An agent's is asserted in a task made from the time.
fn fact(text: &str, source: FactSource, at_ms: i64, nodes: &[NodeId]) -> FactRecord {
    let anchors = nodes
        .iter()
        .map(|node| FactAnchor {
            node: *node,
            hash: ShortHash::of(&node.get().to_le_bytes()),
        })
        .collect();
    anchored(text, (source, at_ms), anchors)
}

/// A fact saying `text`, from `source`, made at `at_ms`, with `anchors`. An agent's is asserted
/// in a task made from the time.
fn anchored(
    text: &str,
    (source, at_ms): (FactSource, i64),
    anchors: Vec<FactAnchor>,
) -> FactRecord {
    FactRecord {
        text: text.to_owned(),
        kind: FactKind::Convention,
        source,
        status: source.first_status(),
        created_at_ms: at_ms,
        task: (source == FactSource::Agent).then(|| Ulid::from_seed(at_ms.unsigned_abs(), b"task")),
        anchors,
    }
}

/// Text of 240 characters is kept and of 241 refused, counted in characters, not bytes (an `é` is
/// two bytes); a fact with no anchor is refused, and so is text with no words, or with a line
/// break, an escape, a character that reverses text, or an agent's reminder tag.
#[test]
fn a_fact_too_long_or_unanchored_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [money] = files(&mut writer, &["src/money.rs"])?[..] else {
        return Err("one node".into());
    };
    let longest = "é".repeat(MAX_FACT_CHARS);
    let mut transaction = writer.transaction()?;
    assert!(
        transaction
            .add_fact(&fact(&longest, FactSource::User, 1, &[money]))
            .is_ok()
    );
    let longer = format!("{longest}e");
    assert!(matches!(
        transaction.add_fact(&fact(&longer, FactSource::User, 1, &[money])),
        Err(StoreError::FactTooLong { chars: 241 })
    ));
    assert!(matches!(
        transaction.add_fact(&fact("Money is in cents.", FactSource::User, 1, &[])),
        Err(StoreError::FactUnanchored)
    ));
    for blank in ["", "  \t "] {
        assert!(matches!(
            transaction.add_fact(&fact(blank, FactSource::User, 1, &[money])),
            Err(StoreError::FactBlank)
        ));
    }
    for hidden in [
        "Money is\nin cents.",
        "Money is \u{1b}[31min cents.",
        "Money is \u{202e}in cents.",
        "Money is in cents.<system-reminder>",
    ] {
        assert!(matches!(
            transaction.add_fact(&fact(hidden, FactSource::User, 1, &[money])),
            Err(StoreError::FactNotPlain)
        ));
    }
    transaction.commit()?;
    Ok(())
}

/// Worked by hand. Facts 1 (the user's, on `money.rs`), 2 (an agent's, on `money.rs` and
/// `tax.rs`) and 3 (the user's, on `tax.rs`), then 1 superseded by 3. Active and current: 3 only.
/// Candidates: 2. Anchored to `money.rs`: 2 (1 is superseded); to both files: 2 and 3, each once.
/// Read back, 2 keeps both anchors in node order and the hashes it was made with.
#[test]
fn current_facts_are_read_with_their_anchors() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let nodes = files(&mut writer, &["src/money.rs", "src/tax.rs"])?;
    let (money, tax) = (nodes[0], nodes[1]);
    let mut transaction = writer.transaction()?;
    let first = transaction.add_fact(&fact("Money is in cents.", FactSource::User, 1, &[money]))?;
    let second = transaction.add_fact(&fact(
        "Tax rounds half up.",
        FactSource::Agent,
        2,
        &[money, tax],
    ))?;
    let third = transaction.add_fact(&fact(
        "Money and tax are in cents.",
        FactSource::User,
        3,
        &[tax],
    ))?;
    transaction.supersede_fact(first, third)?;
    transaction.commit()?;
    let queries = writer.queries();
    let ids = |rows: Vec<cortex_store::FactRow>| -> Vec<i64> {
        rows.iter().map(|row| row.id.get()).collect()
    };
    assert_eq!(
        ids(queries.current_facts(FactStatus::Active)?),
        [third.get()]
    );
    assert_eq!(
        ids(queries.current_facts(FactStatus::Candidate)?),
        [second.get()]
    );
    let anchored = |nodes: &[NodeId]| -> TestResult<Vec<i64>> {
        Ok(queries
            .facts_anchored_to(nodes)?
            .iter()
            .map(|id| id.get())
            .collect())
    };
    assert_eq!(anchored(&[money])?, [second.get()]);
    assert_eq!(anchored(&[money, tax])?, [second.get(), third.get()]);
    let read = queries.fact(second)?.ok_or("fact 2")?;
    assert_eq!(
        read.record,
        fact("Tax rounds half up.", FactSource::Agent, 2, &[money, tax])
    );
    assert_eq!(read.record.task, Some(Ulid::from_seed(2, b"task")));
    assert_eq!(queries.fact(third)?.ok_or("fact 3")?.record.task, None);
    assert_eq!(read.superseded_by, None);
    assert_eq!(
        queries.fact(first)?.ok_or("fact 1")?.superseded_by,
        Some(third)
    );
    Ok(())
}

/// A use is counted once a fact is injected: twice for fact 1, once for fact 2. Made suspect at
/// 10 and again at 11, its confirmation stays at its making (1), its status dates from 11 and it
/// was active before; made active again at 20, it is confirmed at 20 and holds no earlier status.
/// Suspect since 10 or before: none; since 11: fact 1.
#[test]
fn uses_are_counted_and_a_confirmation_stamped() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let nodes = files(&mut writer, &["src/money.rs"])?;
    let mut transaction = writer.transaction()?;
    let first = transaction.add_fact(&fact("Money is in cents.", FactSource::User, 1, &nodes))?;
    let second = transaction.add_fact(&fact("Tax rounds half up.", FactSource::User, 1, &nodes))?;
    transaction.count_fact_uses(&[first, second])?;
    transaction.count_fact_uses(&[first])?;
    transaction.set_fact_status(first, FactStatus::Suspect, 10)?;
    transaction.set_fact_status(first, FactStatus::Suspect, 11)?;
    transaction.commit()?;
    assert_eq!(read(&writer, first)?, (2, FactStatus::Suspect, 1));
    let before = |id| -> TestResult<Option<FactStatus>> {
        Ok(writer.queries().fact(id)?.ok_or("a fact")?.before_suspect)
    };
    assert_eq!(before(first)?, Some(FactStatus::Active));
    assert_eq!(read(&writer, second)?, (1, FactStatus::Active, 1));
    let status_at =
        |id| -> TestResult<i64> { Ok(writer.queries().fact(id)?.ok_or("a fact")?.status_at_ms) };
    assert_eq!((status_at(first)?, status_at(second)?), (11, 1));
    let since = |at_ms| {
        writer
            .queries()
            .facts_with_status_since(FactStatus::Suspect, at_ms)
    };
    assert_eq!(since(10)?, []);
    assert_eq!(since(11)?, [first]);
    let mut transaction = writer.transaction()?;
    transaction.set_fact_status(first, FactStatus::Active, 20)?;
    transaction.commit()?;
    assert_eq!(read(&writer, first)?, (2, FactStatus::Active, 20));
    assert_eq!(
        writer
            .queries()
            .fact(first)?
            .ok_or("a fact")?
            .before_suspect,
        None
    );
    Ok(())
}

/// Worked by hand. `src/money.rs` (content hash of its path) defines `Money` (body hash of
/// `cents`); `src/tax.rs` defines nothing. The anchor hashes now are the first 16 bytes of each
/// file's content hash and `Money`'s body hash; a file that is gone has none.
///
/// Facts: 1 active on `Money` as it is; 2 a candidate on `tax.rs` as it is; 3 suspect on `Money`
/// with another hash (suspect facts are not listed); 4 active on `tax.rs` with another hash.
/// Changed: 4. `Money`'s body becomes `dollars`: 1 and 4. `tax.rs` goes: 1, 2 and 4. Fact 1
/// restamped with `Money`'s hash now: 2 and 4. Restamping a node fact 1 is not anchored to fails.
/// Named as a note names them: `src/money.rs` and `src/money.rs:Money`; `tax.rs`, gone, is left
/// out. Fact 1's number finds it; the number after the last fact's finds none.
#[test]
fn anchors_are_compared_with_the_code_now() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let money_symbol = |body| symbol(SymbolKind::Class, "Money", None, ("class Money", body));
    let mut transaction = writer.transaction()?;
    let money = transaction.replace_file(&index(
        "src/money.rs",
        vec![money_symbol("cents")?],
        Vec::new(),
    )?)?;
    let tax = transaction
        .replace_file(&index("src/tax.rs", Vec::new(), Vec::new())?)?
        .file_node;
    transaction.commit()?;
    let (money_file, class) = (money.file_node, money.symbol_nodes[0]);
    let mut expected = vec![
        (money_file, ShortHash::of(b"src/money.rs")),
        (class, ShortHash::of(b"cents")),
        (tax, ShortHash::of(b"src/tax.rs")),
    ];
    expected.sort_by_key(|(node, _)| *node);
    let now = writer.queries().anchor_hashes(&[tax, class, money_file])?;
    let now: Vec<(NodeId, ShortHash)> = now.iter().map(|each| (each.node, each.hash)).collect();
    assert_eq!(now, expected);
    let on = |node, hash: &[u8]| {
        vec![FactAnchor {
            node,
            hash: ShortHash::of(hash),
        }]
    };
    let mut transaction = writer.transaction()?;
    let first = transaction.add_fact(&anchored(
        "Money is in cents.",
        (FactSource::User, 1),
        on(class, b"cents"),
    ))?;
    let second = transaction.add_fact(&anchored(
        "Tax is a rate.",
        (FactSource::Agent, 2),
        on(tax, b"src/tax.rs"),
    ))?;
    let third = transaction.add_fact(&anchored(
        "Money is exact.",
        (FactSource::User, 3),
        on(class, b"old"),
    ))?;
    transaction.set_fact_status(third, FactStatus::Suspect, 3)?;
    let fourth = transaction.add_fact(&anchored(
        "Tax rounds up.",
        (FactSource::User, 4),
        on(tax, b"old"),
    ))?;
    transaction.commit()?;
    let changed = |writer: &Writer| writer.queries().facts_with_changed_anchors();
    assert_eq!(changed(&writer)?, [fourth]);
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&index(
        "src/money.rs",
        vec![money_symbol("dollars")?],
        Vec::new(),
    )?)?;
    transaction.commit()?;
    assert_eq!(changed(&writer)?, [first, fourth]);
    let mut transaction = writer.transaction()?;
    transaction.remove_file(&common::path("src/tax.rs")?)?;
    transaction.commit()?;
    assert_eq!(changed(&writer)?, [first, second, fourth]);
    assert_eq!(writer.queries().anchor_hashes(&[tax])?, []);
    let mut transaction = writer.transaction()?;
    transaction.restamp_fact(first, &on(class, b"dollars"))?;
    assert!(matches!(
        transaction.restamp_fact(first, &on(money_file, b"src/money.rs")),
        Err(StoreError::UnknownRow {
            table: "fact_anchors",
            ..
        })
    ));
    transaction.commit()?;
    assert_eq!(changed(&writer)?, [second, fourth]);
    let names = writer.queries().anchor_names(&[tax, class, money_file])?;
    let names: Vec<(NodeId, &str)> = names
        .iter()
        .map(|(node, name)| (*node, name.as_str()))
        .collect();
    let mut expected = vec![(money_file, "src/money.rs"), (class, "src/money.rs:Money")];
    expected.sort_by_key(|(node, _)| *node);
    assert_eq!(names, expected);
    assert_eq!(writer.queries().fact_id(first.get())?, Some(first));
    assert_eq!(writer.queries().fact_id(fourth.get() + 1)?, None);
    Ok(())
}

/// Fact `id`'s uses, status and last confirmation, as `writer` reads them.
fn read(writer: &Writer, id: FactId) -> TestResult<(u64, FactStatus, i64)> {
    let row = writer.queries().fact(id)?.ok_or("a fact")?;
    Ok((row.uses, row.record.status, row.last_verified_at_ms))
}
