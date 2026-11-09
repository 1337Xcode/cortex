//! The facts that need a person, counted by hand: those anchored to code the index no longer
//! holds, and those suspect while their code is still there.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{FactKind, FactSource, FactStatus, RepoPath};
use cortex_store::{FactHealth, FactId, FactRecord, NodeId, WriteTx};

/// A fact saying `text`, in force, anchored to `nodes` with the hashes their code has now.
fn add(transaction: &mut WriteTx<'_>, text: &str, nodes: &[NodeId]) -> TestResult<FactId> {
    let anchors = transaction.queries().anchor_hashes(nodes)?;
    Ok(transaction.add_fact(&FactRecord {
        text: text.to_owned(),
        kind: FactKind::Convention,
        source: FactSource::User,
        status: FactStatus::Active,
        created_at_ms: 1,
        task: None,
        anchors,
    })?)
}

/// Worked by hand. `a.py` stays and `b.py` is removed. On `a.py`: fact 1 active, fact 2 suspect,
/// fact 8 active. On `b.py`: fact 3 active, fact 4 suspect, fact 5 a candidate, fact 6 retired,
/// fact 7 superseded by fact 8. Fact 9 on both. Gone: 3, 4, 5 and 9 (6 is retired, 7 superseded;
/// 9 counted once); changed: 2 alone (4 is counted as gone). Before `b.py` is removed, nothing is
/// gone and facts 2 and 4, both suspect, have changed.
#[test]
fn facts_on_code_that_is_gone_or_changed_are_counted_once() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let a = transaction
        .replace_file(&index("a.py", Vec::new(), Vec::new())?)?
        .file_node;
    let b = transaction
        .replace_file(&index("b.py", Vec::new(), Vec::new())?)?
        .file_node;
    add(&mut transaction, "Fact one holds.", &[a])?;
    let two = add(&mut transaction, "Fact two holds.", &[a])?;
    add(&mut transaction, "Fact three holds.", &[b])?;
    let four = add(&mut transaction, "Fact four holds.", &[b])?;
    let five = add(&mut transaction, "Fact five holds.", &[b])?;
    let six = add(&mut transaction, "Fact six holds.", &[b])?;
    let seven = add(&mut transaction, "Fact seven holds.", &[b])?;
    let eight = add(&mut transaction, "Fact seven, corrected.", &[a])?;
    add(&mut transaction, "Fact nine holds.", &[a, b])?;
    transaction.set_fact_status(two, FactStatus::Suspect, 2)?;
    transaction.set_fact_status(four, FactStatus::Suspect, 2)?;
    transaction.set_fact_status(five, FactStatus::Candidate, 2)?;
    transaction.set_fact_status(six, FactStatus::Retired, 2)?;
    transaction.supersede_fact(seven, eight)?;
    assert_eq!(
        transaction.queries().fact_health()?,
        FactHealth {
            gone: 0,
            changed: 2
        }
    );
    transaction.remove_file(&RepoPath::new("b.py")?)?;
    assert_eq!(
        transaction.queries().fact_health()?,
        FactHealth {
            gone: 4,
            changed: 1
        }
    );
    transaction.commit()?;
    Ok(())
}
