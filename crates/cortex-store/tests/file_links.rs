//! How a file is linked to the others: the strongest edge of each kind between it, or a symbol in
//! it, and each other file, read both ways, worked out by hand.

mod common;

use common::{Scratch, TestResult, index, symbol};
use cortex_core::{EdgeType, RepoPath, SymbolKind};
use cortex_store::{EdgeRecord, FileLink, NodeId};

/// An undirected edge to `dst` of `weight`, seen once.
fn edge(dst: NodeId, weight: f64) -> EdgeRecord {
    EdgeRecord {
        dst,
        weight,
        count: 1,
        last_seq: 0,
        is_directed: false,
    }
}

/// Worked by hand. `a.py` defines `f`, `b.py` defines `g`, `c.py` defines nothing. Edges: `f` to
/// `g`, structural, 0.8, and 0.3 from `a.py` to `b.py`; `a.py` to `c.py` and back, co-change, 0.6
/// and 0.5; `b.py` to `a.py`, co-access, 0.4; `a.py` to its own `f`, structural, 0.9. The links of
/// `a.py`: by code to `b.py`, 0.8 (the stronger); by history to `c.py`, 0.6; by use to `b.py`, 0.4,
/// read the other way. Its link to its own symbol is not another file's.
#[test]
fn a_file_s_links_are_the_strongest_of_each_kind() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let def = |name: &str| symbol(SymbolKind::Function, name, None, (name, name));
    let a = transaction.replace_file(&index("a.py", vec![def("f")?], Vec::new())?)?;
    let b = transaction.replace_file(&index("b.py", vec![def("g")?], Vec::new())?)?;
    let c = transaction.replace_file(&index("c.py", Vec::new(), Vec::new())?)?;
    let (a_file, f) = (a.file_node, a.symbol_nodes[0]);
    let (b_file, g) = (b.file_node, b.symbol_nodes[0]);
    let c_file = c.file_node;
    transaction.replace_edges(f, EdgeType::Structural, &[edge(g, 0.8)])?;
    transaction.replace_edges(
        a_file,
        EdgeType::Structural,
        &[edge(b_file, 0.3), edge(f, 0.9)],
    )?;
    transaction.replace_edges(a_file, EdgeType::CoChange, &[edge(c_file, 0.6)])?;
    transaction.replace_edges(c_file, EdgeType::CoChange, &[edge(a_file, 0.5)])?;
    transaction.replace_edges(b_file, EdgeType::CoAccess, &[edge(a_file, 0.4)])?;
    transaction.commit()?;
    let path = |text: &str| RepoPath::new(text);
    assert_eq!(
        writer.queries().links_of_file(a_file)?,
        [
            FileLink {
                path: path("b.py")?,
                edge_type: EdgeType::Structural,
                weight: 0.8,
            },
            FileLink {
                path: path("c.py")?,
                edge_type: EdgeType::CoChange,
                weight: 0.6,
            },
            FileLink {
                path: path("b.py")?,
                edge_type: EdgeType::CoAccess,
                weight: 0.4,
            },
        ]
    );
    assert_eq!(writer.queries().links_of_file(f)?, []);
    Ok(())
}
