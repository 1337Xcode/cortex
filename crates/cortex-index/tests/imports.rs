//! Import edges after a full index of small repositories: each parsed file is joined to the files
//! its imports name, as the extractor reads the imports and each language family resolves them,
//! and a later index follows the changes.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use common::{TestResult, index, lock_out, write};
use cortex_core::{EdgeType, RepoPath};
use cortex_index::{Problem, ProblemKind};
use cortex_store::{NodeId, Reader};

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// The strength of a reference tied by its import path, which an import edge carries.
const IMPORT_PATH_CONFIDENCE: f64 = 0.8;

/// One importer's edges: its path, then each file its imports name with how many of them do.
type Edges = (String, Vec<(String, u32)>);

/// Writes each file of `files`, by path and text, under `root`.
fn write_all(root: &Path, files: &[(&str, &str)]) -> TestResult {
    for (path, text) in files {
        write(root, path, text.as_bytes())?;
    }
    Ok(())
}

/// Each file of the repository at `root` with structural edges, in path order, with the files
/// its edges reach in path order and the count of each. Every structural edge is checked to
/// point one way, carry the import path's strength and have no task behind it.
fn import_edges(root: &Path) -> TestResult<Vec<Edges>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let queries = reader.queries();
    let files = queries.file_nodes()?;
    let paths: BTreeMap<NodeId, String> = files
        .iter()
        .map(|(path, node)| (*node, path.as_str().to_owned()))
        .collect();
    let mut found = Vec::new();
    for (path, node) in &files {
        let mut targets = Vec::new();
        for row in queries.edges_from(*node)? {
            if row.edge_type != EdgeType::Structural {
                continue;
            }
            let edge = row.edge;
            assert!(
                edge.weight.total_cmp(&IMPORT_PATH_CONFIDENCE).is_eq(),
                "{path:?}"
            );
            assert_eq!((edge.last_seq, edge.is_directed), (0, true), "{path:?}");
            let target = paths.get(&edge.dst).ok_or("an edge to no file")?;
            targets.push((target.clone(), edge.count));
        }
        if !targets.is_empty() {
            targets.sort();
            found.push((path.as_str().to_owned(), targets));
        }
    }
    Ok(found)
}

/// The expected edges: each importer with the files it reaches and the count of each.
fn edges(expected: &[(&str, &[(&str, u32)])]) -> Vec<Edges> {
    expected
        .iter()
        .map(|(importer, targets)| {
            let targets = targets
                .iter()
                .map(|(target, count)| ((*target).to_owned(), *count))
                .collect();
            ((*importer).to_owned(), targets)
        })
        .collect()
}

/// TypeScript and JavaScript imports join files through relative paths, a tsconfig.json pattern,
/// a re-export and `require`; a package names nothing. Two names from one module count twice. A
/// later index moves an edge to a file that took its target's place, drops the edges of imports
/// that are gone, and keeps the edges of a file it could not read.
#[test]
fn typescript_and_javascript_imports_join_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (
                "tsconfig.json",
                r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": ["src/*"]}}}"#,
            ),
            (
                "src/app.ts",
                "import { total } from \"./util\";\nimport { Order, place } from \"@/models/order\";\nimport React from \"react\";\nimport \"./styles.css\";\nexport { format } from \"./format\";\n",
            ),
            ("src/util.ts", "export function total() { return 1; }\n"),
            (
                "src/models/order.ts",
                "export class Order {}\nexport function place() {}\n",
            ),
            ("src/styles.css", "body { margin: 0; }\n"),
            (
                "src/format.js",
                "const { Order } = require(\"./models/order\");\nexport function format() { return new Order(); }\n",
            ),
            ("src/old.ts", "import { Order } from \"./models/order\";\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let first: &[(&str, &[(&str, u32)])] = &[
        (
            "src/app.ts",
            &[
                ("src/format.js", 1),
                ("src/models/order.ts", 2),
                ("src/styles.css", 1),
                ("src/util.ts", 1),
            ],
        ),
        ("src/format.js", &[("src/models/order.ts", 1)]),
        ("src/old.ts", &[("src/models/order.ts", 1)]),
    ];
    assert_eq!(import_edges(root)?, edges(first));

    std::fs::remove_file(root.join("src/util.ts"))?;
    write_all(
        root,
        &[
            ("src/util.tsx", "export function total() { return 1; }\n"),
            (
                "src/app.ts",
                "import { total } from \"./util\";\nimport { Order, place } from \"@/models/order\";\nexport { format } from \"./format\";\n",
            ),
            ("src/old.ts", "export const nothing = 0;\n"),
        ],
    )?;
    let report = {
        let _locked = lock_out(&root.join("src/format.js"))?;
        index(root, 2, GENEROUS, DEFAULT_LIMIT)?
    };
    let unreadable = Problem {
        path: Some(RepoPath::new("src/format.js")?),
        kind: ProblemKind::Unreadable,
    };
    assert_eq!(report.walk_problems, [unreadable]);
    let second: &[(&str, &[(&str, u32)])] = &[
        (
            "src/app.ts",
            &[
                ("src/format.js", 1),
                ("src/models/order.ts", 2),
                ("src/util.tsx", 1),
            ],
        ),
        ("src/format.js", &[("src/models/order.ts", 1)]),
    ];
    assert_eq!(import_edges(root)?, edges(second));
    Ok(())
}

/// A config that extends a file that may hold secrets cannot be followed, since that file is never
/// opened: the specifier its `baseUrl` would have resolved names nothing, and a relative one still
/// resolves.
#[test]
fn a_config_is_not_followed_into_a_file_that_may_hold_secrets() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("tsconfig.json", r#"{"extends": "./credentials.json"}"#),
            (
                "credentials.json",
                r#"{"compilerOptions": {"baseUrl": "."}}"#,
            ),
            (
                "src/main.ts",
                "import { x } from \"lib/x\";\nimport { y } from \"./y\";\n",
            ),
            ("lib/x.ts", "export const x = 1;\n"),
            ("src/y.ts", "export const y = 2;\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let expected: &[(&str, &[(&str, u32)])] = &[("src/main.ts", &[("src/y.ts", 1)])];
    assert_eq!(import_edges(root)?, edges(expected));
    Ok(())
}

/// Python imports join files through relative imports, a package's submodule and a source root
/// beside pyproject.toml; the standard library names nothing, even with a `json.py` beside the
/// importer in its package.
#[test]
fn python_imports_join_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("pyproject.toml", "[project]\nname = \"shop\"\n"),
            (
                "src/shop/__init__.py",
                "from . import orders\nfrom .orders import Order\n",
            ),
            (
                "src/shop/orders.py",
                "from .money import total, rate\nimport os\nimport json\n\nclass Order:\n    pass\n",
            ),
            (
                "src/shop/money.py",
                "def total():\n    pass\n\ndef rate():\n    pass\n",
            ),
            ("src/shop/json.py", "def dumps():\n    pass\n"),
            (
                "tests/test_orders.py",
                "from shop.orders import Order\nimport shop.money\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let expected: &[(&str, &[(&str, u32)])] = &[
        ("src/shop/__init__.py", &[("src/shop/orders.py", 2)]),
        ("src/shop/orders.py", &[("src/shop/money.py", 2)]),
        (
            "tests/test_orders.py",
            &[("src/shop/money.py", 1), ("src/shop/orders.py", 1)],
        ),
    ];
    assert_eq!(import_edges(root)?, edges(expected));
    Ok(())
}

/// A Go import joins the importer to every file of the package, its tests left out, through the
/// module path go.mod declares, renamed or not; the standard library names nothing.
#[test]
fn go_imports_join_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("go.mod", "module example.com/shop\n\ngo 1.22\n"),
            (
                "cmd/shop/main.go",
                "package main\n\nimport (\n\t\"fmt\"\n\t\"example.com/shop/orders\"\n\tdb \"example.com/shop/internal/store\"\n)\n\nfunc main() { fmt.Println(orders.Place(), db.Open()) }\n",
            ),
            (
                "orders/order.go",
                "package orders\n\nfunc Place() int { return 1 }\n",
            ),
            (
                "orders/money.go",
                "package orders\n\nfunc total() int { return 2 }\n",
            ),
            ("orders/order_test.go", "package orders\n"),
            (
                "internal/store/store.go",
                "package store\n\nfunc Open() int { return 0 }\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let expected: &[(&str, &[(&str, u32)])] = &[(
        "cmd/shop/main.go",
        &[
            ("internal/store/store.go", 1),
            ("orders/money.go", 1),
            ("orders/order.go", 1),
        ],
    )];
    assert_eq!(import_edges(root)?, edges(expected));
    Ok(())
}

/// Rust `use` paths join files through `crate`, `super`, a crate root's own modules, a module
/// named as a whole and the package's library by name; the standard library names nothing, a
/// `mod` declaration is not an import, and with `src/lib.rs` beside `src/main.rs` a path that ends
/// at the crate root names only the module it takes.
#[test]
fn rust_imports_join_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"shop-core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            (
                "src/lib.rs",
                "pub mod models;\nmod api;\npub use models::Order;\n",
            ),
            ("src/models.rs", "pub struct Order;\n"),
            ("src/api/mod.rs", "mod routes;\nuse crate::models::Order;\n"),
            (
                "src/api/routes.rs",
                "use super::*;\nuse crate::models;\nuse std::collections::HashMap;\n",
            ),
            ("src/main.rs", "use shop_core::Order;\n\nfn main() {}\n"),
            (
                "tests/flow.rs",
                "mod common;\nuse common::setup;\n\n#[test]\nfn flows() {\n    setup();\n}\n",
            ),
            ("tests/common/mod.rs", "pub fn setup() {}\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let expected: &[(&str, &[(&str, u32)])] = &[
        ("src/api/mod.rs", &[("src/models.rs", 1)]),
        (
            "src/api/routes.rs",
            &[("src/api/mod.rs", 1), ("src/models.rs", 1)],
        ),
        ("src/lib.rs", &[("src/models.rs", 1)]),
        ("src/main.rs", &[("src/lib.rs", 1)]),
        ("tests/flow.rs", &[("tests/common/mod.rs", 1)]),
    ];
    assert_eq!(import_edges(root)?, edges(expected));
    Ok(())
}
