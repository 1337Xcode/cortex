//! Facts as the store keeps them: refused when they can never be held (too long, no anchor), read
//! back with their anchors, current until superseded, found by the nodes they are anchored to,
//! their uses counted and their confirmation stamped when made active again.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{ContentHash, FactKind, FactSource, FactStatus, MAX_FACT_CHARS};
use cortex_store::{FactAnchor, FactRecord, NodeId, StoreError, Writer};

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
/// made from its id.
fn fact(text: &str, source: FactSource, at_ms: i64, nodes: &[NodeId]) -> FactRecord {
    FactRecord {
        text: text.to_owned(),
        kind: FactKind::Convention,
        source,
        status: source.first_status(),
        created_at_ms: at_ms,
        anchors: nodes
            .iter()
            .map(|node| FactAnchor {
                node: *node,
                hash: ContentHash::of(&node.get().to_le_bytes()),
            })
            .collect(),
    }
}

/// Text of 240 characters is kept and of 241 refused, counted in characters, not bytes (an `é` is
/// two bytes); a fact with no anchor is refused.
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
    assert_eq!(read.superseded_by, None);
    assert_eq!(
        queries.fact(first)?.ok_or("fact 1")?.superseded_by,
        Some(third)
    );
    Ok(())
}

/// A use is counted once a fact is injected: twice for fact 1, once for fact 2. Made suspect at
/// 10, its confirmation stays at its making (1); made active again at 20, it is confirmed at 20.
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
    transaction.commit()?;
    assert_eq!(read(&writer, first)?, (2, FactStatus::Suspect, 1));
    assert_eq!(read(&writer, second)?, (1, FactStatus::Active, 1));
    let mut transaction = writer.transaction()?;
    transaction.set_fact_status(first, FactStatus::Active, 20)?;
    transaction.commit()?;
    assert_eq!(read(&writer, first)?, (2, FactStatus::Active, 20));
    Ok(())
}

/// Fact `id`'s uses, status and last confirmation, as `writer` reads them.
fn read(writer: &Writer, id: cortex_store::FactId) -> TestResult<(u64, FactStatus, i64)> {
    let row = writer.queries().fact(id)?.ok_or("a fact")?;
    Ok((row.uses, row.record.status, row.last_verified_at_ms))
}
