//! What the self-check last found, kept in the `meta` table and read back.

mod common;

use common::{Scratch, TestResult};
use cortex_store::{SelfCheckRecord, StoreError};

/// Before the self-check first ran it reads as all zero; what it found reads back as kept, and
/// what it found next replaces it; a stored value that cannot be read back is an error naming its
/// key, not zero.
#[test]
fn the_self_check_is_kept_and_read_back() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    assert_eq!(writer.queries().self_check()?, SelfCheckRecord::default());
    let first = SelfCheckRecord {
        raise_steps: 1,
        raised_at: 100,
        recent_labelled: 100,
        recent_used: 59,
    };
    let next = SelfCheckRecord {
        recent_used: 70,
        ..first
    };
    for record in [first, next] {
        let mut transaction = writer.transaction()?;
        transaction.set_self_check(&record)?;
        transaction.commit()?;
        assert_eq!(writer.queries().self_check()?, record);
    }
    drop(writer);
    rusqlite::Connection::open(scratch.folder.join("cortex.db"))?
        .execute_batch("UPDATE meta SET value = 'raised' WHERE key = 'self_check'")?;
    let read = scratch.reader()?.queries().self_check();
    assert!(
        matches!(read, Err(StoreError::CorruptMeta { key: "self_check" })),
        "{read:?}"
    );
    Ok(())
}
