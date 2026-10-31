//! The graph's structural edges after a full index of small repositories: between definitions, by
//! the references tied from one to another, and from a test file to the file it tests.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use common::{TestResult, index, write_all};
use cortex_core::EdgeType;
use cortex_store::{NodeId, Reader};

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// One structural edge as a test reads it: where it starts and ends, each a file path and, for a
/// definition, its qualified name, then its weight and count.
type Edge = (String, String, f64, u32);

/// Every structural edge from the nodes of the repository at `root`, each end named by its file
/// and, for a definition, `path#qualified name`, in order.
fn structural_edges(root: &Path) -> TestResult<Vec<Edge>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let queries = reader.queries();
    let mut names: BTreeMap<NodeId, String> = BTreeMap::new();
    for (path, node) in queries.file_nodes()? {
        names.insert(node, path.as_str().to_owned());
    }
    for (path, file) in queries.file_ids()? {
        for symbol in queries.symbols_in_file(file)? {
            if let Some(node) = queries.symbol_node(symbol.id)? {
                names.insert(
                    node,
                    format!("{}#{}", path.as_str(), symbol.qual_name.as_str()),
                );
            }
        }
    }
    let mut edges = Vec::new();
    for (node, from) in &names {
        for row in queries.edges_from(*node)? {
            if row.edge_type != EdgeType::Structural {
                continue;
            }
            let to = names.get(&row.edge.dst).ok_or("an edge to no named node")?;
            edges.push((from.clone(), to.clone(), row.edge.weight, row.edge.count));
        }
    }
    edges.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    Ok(edges)
}

/// The edges among `edges` that start at a definition.
fn from_definitions(edges: Vec<Edge>) -> Vec<Edge> {
    edges
        .into_iter()
        .filter(|edge| edge.0.contains('#'))
        .collect()
}

/// The expected edges, by hand.
fn edges(expected: &[(&str, &str, f64, u32)]) -> Vec<Edge> {
    expected
        .iter()
        .map(|(from, to, weight, count)| ((*from).to_owned(), (*to).to_owned(), *weight, *count))
        .collect()
}

/// Definitions are joined by the references tied from one to another, each edge as strong as its
/// strongest reference and counting them. In `app.py`, `Child` inherits the imported `Base`
/// (0.8, line 4); `Child.run` calls the imported `helper` twice (0.8, lines 6 and 7); `main` calls
/// `local` in its own file (0.6) and itself, which makes no edge. The imports at the top of the
/// file sit in no definition and make none. Indexed again with `run`'s calls gone, `Child.run`
/// loses its edge.
#[test]
fn definitions_are_joined_by_their_ties() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    let app = "from lib import helper, Base\n\n\nclass Child(Base):\n    def run(self):\n        \
               helper()\n        helper()\n\n\ndef main():\n    local()\n    main()\n\n\n\
               def local():\n    pass\n";
    write_all(
        root,
        &[
            (
                "lib.py",
                "def helper():\n    pass\n\n\nclass Base:\n    pass\n",
            ),
            ("app.py", app),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let base = ("app.py#Child", "lib.py#Base", 0.8, 1);
    let local = ("app.py#main", "app.py#local", 0.6, 1);
    assert_eq!(
        from_definitions(structural_edges(root)?),
        edges(&[base, ("app.py#Child.run", "lib.py#helper", 0.8, 2), local])
    );
    write_all(
        root,
        &[(
            "app.py",
            &app.replace("        helper()\n        helper()\n", "        pass\n"),
        )],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        from_definitions(structural_edges(root)?),
        edges(&[base, local])
    );
    Ok(())
}

/// A test file whose imports name no source file gets an edge to the file its name spells without
/// the test marks, as strong as a tie by name (0.3): the Go test beside `orders.go`, which imports
/// only the standard library, and the TypeScript test importing only a package. `test_util.py`
/// could test `a/util.py` or `b/util.py`, neither beside it, and gets none. `test_shop.py` imports
/// `shop.py` and has only its import edge (0.8).
#[test]
fn tests_are_joined_to_what_they_test() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("go.mod", "module example.com/shop\n"),
            ("pkg/orders.go", "package pkg\n\nfunc Place() {}\n"),
            (
                "pkg/orders_test.go",
                "package pkg\n\nimport \"testing\"\n\nfunc TestPlace(t *testing.T) {\n\tPlace()\n}\n",
            ),
            ("src/cart.ts", "export function add() {}\n"),
            (
                "src/cart.test.ts",
                "import { describe } from \"vitest\";\ndescribe();\n",
            ),
            ("a/util.py", "def one():\n    pass\n"),
            ("b/util.py", "def two():\n    pass\n"),
            ("tests/test_util.py", "def test_one():\n    pass\n"),
            ("shop.py", "class Order:\n    pass\n"),
            ("tests/test_shop.py", "from shop import Order\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let from_files: Vec<Edge> = structural_edges(root)?
        .into_iter()
        .filter(|edge| !edge.0.contains('#'))
        .collect();
    assert_eq!(
        from_files,
        edges(&[
            ("pkg/orders_test.go", "pkg/orders.go", 0.3, 1),
            ("src/cart.test.ts", "src/cart.ts", 0.3, 1),
            ("tests/test_shop.py", "shop.py", 0.8, 1),
        ])
    );
    Ok(())
}
