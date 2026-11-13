//! A file panel's symbols read through the site: which symbols the work orders' spans share lines
//! with, the spans themselves, and the cap on how many symbols are listed.

use cortex_core::SymbolKind;
use serde_json::{Value, json};

use super::script::{file, first_task, script, symbol, work_order};
use super::{get, site_over};
use crate::test_store::{TestResult, writer};

/// How many symbols `src/big.rs` defines: one more than the panel lists.
const DEFINED: u32 = 501;

/// The panel of `src/big.rs`, id 2 among the scripted files sorted by path, as JSON. The file
/// defines functions `s1` to `s501`, `sn` on lines 10n to 10n + 1. Two delivered work orders named
/// it: the first selected lines 10 to 10 and 20 to 25, the second 20 to 20 and 5,010 to 5,011.
fn big_panel() -> TestResult<Value> {
    let root = tempfile::tempdir()?;
    let texts = script(root.path())?;
    let mut writer = writer(root.path())?;
    let mut transaction = writer.transaction()?;
    let symbols = (1..=DEFINED)
        .map(|n| symbol(&format!("s{n}"), SymbolKind::Function, n * 10))
        .collect::<TestResult<Vec<_>>>()?;
    let big = transaction.replace_file(&file("src/big.rs", 5_020, symbols)?)?;
    let task = (first_task(), "s1");
    for (at_ms, spans) in [
        (1_700, vec![(10, 10), (20, 25)]),
        (1_800, vec![(20, 20), (5_010, 5_011)]),
    ] {
        let mut order = work_order(task, at_ms, &[big.file_node])?;
        for named in &mut order.files {
            named.spans.clone_from(&spans);
        }
        transaction.record_injection(&order)?;
    }
    transaction.commit()?;
    drop(writer);
    let fixture = site_over(root, texts)?;
    let (status, body) = get(&fixture, "api/file/2")?;
    assert_eq!(status, 200, "{body}");
    let panel: Value = serde_json::from_str(&body)?;
    assert_eq!(panel["path"], "src/big.rs");
    Ok(panel)
}

/// `s1` (lines 10 to 11) shares line 10 with the first order only: 1. `s2` (20 to 21) shares lines
/// with a span of each order: 2. `s3` (30 to 31) with none: 0. The spans are listed by their lines,
/// each selected by one order; the last selects `s501`, which is past the 500 listed.
#[test]
fn a_symbol_counts_the_work_orders_that_selected_its_lines() -> TestResult {
    let panel = big_panel()?;
    let symbols = panel["symbols"].as_array().ok_or("no symbols")?;
    let selected: Vec<(&str, u64)> = symbols
        .iter()
        .take(3)
        .filter_map(|entry| Some((entry["name"].as_str()?, entry["selected"].as_u64()?)))
        .collect();
    assert_eq!(selected, [("s1", 1), ("s2", 2), ("s3", 0)]);
    assert_eq!(
        symbols[0],
        json!({"name": "s1", "kind": "function", "start_line": 10, "end_line": 11, "is_nested": false, "selected": 1})
    );
    assert_eq!(
        panel["selected"],
        json!([
            {"start_line": 10, "end_line": 10, "orders": 1},
            {"start_line": 20, "end_line": 20, "orders": 1},
            {"start_line": 20, "end_line": 25, "orders": 1},
            {"start_line": 5_010, "end_line": 5_011, "orders": 1},
        ])
    );
    Ok(())
}

/// Of the 501 symbols, the first 500 are listed, `s1` to `s500` in line order, and the total says
/// 501.
#[test]
fn a_file_lists_its_first_five_hundred_symbols_and_counts_them_all() -> TestResult {
    let panel = big_panel()?;
    let names: Vec<&str> = panel["symbols"]
        .as_array()
        .ok_or("no symbols")?
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    let expected: Vec<String> = (1..=500).map(|n| format!("s{n}")).collect();
    assert_eq!(names, expected);
    assert_eq!(panel["symbols_total"], 501);
    Ok(())
}
