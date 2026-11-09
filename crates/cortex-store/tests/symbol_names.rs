//! Finding symbols by the name given: the qualified name first, then the plain name, worked out by
//! hand.

mod common;

use common::{Scratch, TestResult, index, symbol};
use cortex_core::SymbolKind;
use cortex_store::SymbolRow;

/// The qualified names of `rows`, in their order.
fn qualified(rows: &[SymbolRow]) -> Vec<&str> {
    rows.iter().map(|row| row.qual_name.as_str()).collect()
}

/// Worked by hand. `a.py` defines `Order.create` and a function `create`; `b.py` defines
/// `Cart.create`; `c.py` defines `Tax.rate` and `Fee.rate`. `create` is the function's qualified
/// name, so in `a.py` and in the whole repository it names the function alone, not the methods
/// whose plain name it is; in `b.py`, where nothing is qualified so, it names `Cart.create`.
/// `rate` is no symbol's qualified name, so it names both methods, oldest first. A qualified name
/// names its symbol, and a name nothing has names nothing.
#[test]
fn a_name_given_is_a_qualified_name_first_then_a_plain_one() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let def = |kind, qual: &str| symbol(kind, qual, None, (qual, qual));
    let a = transaction.replace_file(&index(
        "a.py",
        vec![
            def(SymbolKind::Method, "Order.create")?,
            def(SymbolKind::Function, "create")?,
        ],
        Vec::new(),
    )?)?;
    let b = transaction.replace_file(&index(
        "b.py",
        vec![def(SymbolKind::Method, "Cart.create")?],
        Vec::new(),
    )?)?;
    transaction.replace_file(&index(
        "c.py",
        vec![
            def(SymbolKind::Method, "Tax.rate")?,
            def(SymbolKind::Method, "Fee.rate")?,
        ],
        Vec::new(),
    )?)?;
    transaction.commit()?;
    let queries = writer.queries();
    let file_of = |node| -> TestResult<_> { Ok(queries.node_file(node)?.ok_or("no file")?.0) };
    let (a, b) = (file_of(a.file_node)?, file_of(b.file_node)?);
    assert_eq!(
        qualified(&queries.symbols_in_file_called(a, "create")?),
        ["create"]
    );
    assert_eq!(
        qualified(&queries.symbols_in_file_called(a, "Order.create")?),
        ["Order.create"]
    );
    assert_eq!(
        qualified(&queries.symbols_in_file_called(b, "create")?),
        ["Cart.create"]
    );
    assert_eq!(qualified(&queries.symbols_called("create")?), ["create"]);
    assert_eq!(
        qualified(&queries.symbols_called("Cart.create")?),
        ["Cart.create"]
    );
    assert_eq!(
        qualified(&queries.symbols_called("rate")?),
        ["Tax.rate", "Fee.rate"]
    );
    assert_eq!(
        qualified(&queries.symbols_called("total")?),
        Vec::<&str>::new()
    );
    Ok(())
}
