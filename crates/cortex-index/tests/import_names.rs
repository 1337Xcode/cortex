//! References tied through imports after a full index of small repositories: a name an import
//! binds is tied to the definition the import names, following files that only pass the name on,
//! and an import's own reference is tied that way or not at all.

mod common;

use std::time::Duration;

use common::{TestResult, expected, index, lock_out, ties, write, write_all};
use cortex_core::RepoPath;

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// TypeScript names tied through imports. `app.ts` takes `Order` and `Line` from the index file
/// of `models`, which passes `Order` on from `order.ts` by name and everything of `line.ts` with
/// `*`; it takes `place` from `order.ts` under the name `put`, `useState` from a package, and the
/// default export of `widget.ts`. The index file's own import reference is tied too. An import of
/// a package's name stays unresolved, and so does the name it binds, which is the package's,
/// though the language family has a `useState` of its own. A default import binds nothing, so
/// `Widget` goes by its name.
#[test]
fn typescript_names_are_tied_through_imports() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (
                "src/models/order.ts",
                "export class Order {}\nexport function place(order: Order) {}\n",
            ),
            ("src/models/line.ts", "export class Line {}\n"),
            (
                "src/models/index.ts",
                "export { Order } from \"./order\";\nexport * from \"./line\";\n",
            ),
            ("src/widget.ts", "export default class Widget {}\n"),
            ("src/hooks.ts", "export function useState() {}\n"),
            (
                "src/app.ts",
                "import { Order, Line } from \"./models\";\n\
                 import { place as put } from \"./models/order\";\n\
                 import { useState } from \"react\";\n\
                 import Widget from \"./widget\";\n\
                 put(new Order());\n\
                 useState(new Line(), new Widget());\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let order = Some(("src/models/order.ts", "Order", "import_path"));
    let line = Some(("src/models/line.ts", "Line", "import_path"));
    let place = Some(("src/models/order.ts", "place", "import_path"));
    assert_eq!(
        ties(root, "src/app.ts")?,
        expected(&[
            ("import", "Order", 1, order),
            ("import", "Line", 1, line),
            ("import", "place", 2, place),
            ("import", "useState", 3, None),
            ("call", "put", 5, place),
            ("call", "Order", 5, order),
            ("call", "useState", 6, None),
            ("call", "Line", 6, line),
            (
                "call",
                "Widget",
                6,
                Some(("src/widget.ts", "Widget", "name_only"))
            ),
        ])
    );
    assert_eq!(
        ties(root, "src/models/index.ts")?,
        expected(&[("import", "Order", 1, order)])
    );
    Ok(())
}

/// Python names tied through a package. `shop/__init__.py` passes `Order` on from `models.py` and
/// everything of `core.py`. `app.py` takes `f` from two modules, so its import references are tied
/// one to each, while the name `f` they both bind finds two definitions and stays unresolved, as
/// it does by name. It takes `Order` twice, through the package and from the module, which find
/// the same class. `helper` is only a method of that class, no name of the module, and `List`,
/// from the standard library, is not in the repository: both stay unresolved.
#[test]
fn python_names_are_tied_through_packages() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (
                "shop/__init__.py",
                "from .models import Order\nfrom .core import *\n",
            ),
            (
                "shop/models.py",
                "class Order:\n    def helper(self):\n        pass\n",
            ),
            ("shop/core.py", "def total():\n    return 0\n"),
            ("shop/a.py", "def f():\n    pass\n"),
            ("shop/b.py", "def f():\n    pass\n"),
            (
                "app.py",
                "from shop import Order, total\nfrom shop.a import f\nfrom shop.b import f\n\
                 from typing import List\nfrom shop.models import Order, helper\n\n\
                 Order()\ntotal()\nf()\nList()\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let order = Some(("shop/models.py", "Order", "import_path"));
    let total = Some(("shop/core.py", "total", "import_path"));
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "Order", 1, order),
            ("import", "total", 1, total),
            ("import", "f", 2, Some(("shop/a.py", "f", "import_path"))),
            ("import", "f", 3, Some(("shop/b.py", "f", "import_path"))),
            ("import", "List", 4, None),
            ("import", "Order", 5, order),
            ("import", "helper", 5, None),
            ("call", "Order", 7, order),
            ("call", "total", 8, total),
            ("call", "f", 9, None),
            ("call", "List", 10, None),
        ])
    );
    assert_eq!(
        ties(root, "shop/__init__.py")?,
        expected(&[("import", "Order", 1, order)])
    );
    Ok(())
}

/// Rust names tied through `use`: a type taken from a module, and a module taken from the crate
/// root, whose import reference is tied to its `mod` declaration. A method of an `impl` block is
/// no name of the module: taking `new` from it finds nothing, so the call `Order::new()` goes by
/// its name, to the only `new` of the language family.
#[test]
fn rust_names_are_tied_through_use() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("Cargo.toml", "[package]\nname = \"shop\"\n"),
            ("src/lib.rs", "pub mod models;\npub mod api;\n"),
            (
                "src/models.rs",
                "pub struct Order;\n\nimpl Order {\n    pub fn new() -> Self {\n        Order\n    }\n}\n",
            ),
            (
                "src/api.rs",
                "use crate::models::Order;\nuse crate::models;\nuse crate::models::new;\n\n\
                 pub fn make() -> Order {\n    Order::new()\n}\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let order = Some(("src/models.rs", "Order", "import_path"));
    assert_eq!(
        ties(root, "src/api.rs")?,
        expected(&[
            ("import", "Order", 1, order),
            (
                "import",
                "models",
                2,
                Some(("src/lib.rs", "models", "import_path"))
            ),
            ("import", "new", 3, None),
            ("type_ref", "Order", 5, order),
            (
                "call",
                "new",
                6,
                Some(("src/models.rs", "Order.new", "name_only"))
            ),
        ])
    );
    Ok(())
}

/// A name is followed through at most four files that only pass it on: `Deep` passes through
/// four and is found, `Deeper` through five and is not. Two files that pass a name to each other
/// end the search with nothing found.
#[test]
fn names_pass_through_at_most_four_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    let mut files = vec![
        ("src/c5.ts".to_owned(), "export class Deep {}\n".to_owned()),
        (
            "src/d6.ts".to_owned(),
            "export class Deeper {}\n".to_owned(),
        ),
        (
            "src/x.ts".to_owned(),
            "export { Loop } from \"./y\";\n".to_owned(),
        ),
        (
            "src/y.ts".to_owned(),
            "export { Loop } from \"./x\";\n".to_owned(),
        ),
        (
            "src/use.ts".to_owned(),
            "import { Deep } from \"./c1\";\nimport { Deeper } from \"./d1\";\n\
             import { Loop } from \"./x\";\n"
                .to_owned(),
        ),
    ];
    for step in 1..=4 {
        let text = format!("export {{ Deep }} from \"./c{}\";\n", step + 1);
        files.push((format!("src/c{step}.ts"), text));
    }
    for step in 1..=5 {
        let text = format!("export {{ Deeper }} from \"./d{}\";\n", step + 1);
        files.push((format!("src/d{step}.ts"), text));
    }
    for (path, text) in &files {
        write(root, path, text.as_bytes())?;
    }
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "src/use.ts")?,
        expected(&[
            (
                "import",
                "Deep",
                1,
                Some(("src/c5.ts", "Deep", "import_path"))
            ),
            ("import", "Deeper", 2, None),
            ("import", "Loop", 3, None),
        ])
    );
    Ok(())
}

/// A file reached the long way, where the cap cut the search short, is searched again when a
/// shorter way reaches it. `hub.ts` brings in everything of `b1.ts`, which passes `Diamond` on
/// through `b2.ts` and `b3.ts` to `f.ts`, the fourth file passed through, where the search stops;
/// `hub.ts` also brings in everything of `f.ts` directly, and from there `f.ts` passes `Diamond`
/// on from `g.ts`, which makes it.
#[test]
fn a_shorter_way_to_a_file_is_followed_again() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (
                "src/hub.ts",
                "export * from \"./b1\";\nexport * from \"./f\";\n",
            ),
            ("src/b1.ts", "export { Diamond } from \"./b2\";\n"),
            ("src/b2.ts", "export { Diamond } from \"./b3\";\n"),
            ("src/b3.ts", "export { Diamond } from \"./f\";\n"),
            ("src/f.ts", "export { Diamond } from \"./g\";\n"),
            ("src/g.ts", "export class Diamond {}\n"),
            ("src/use.ts", "import { Diamond } from \"./hub\";\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "src/use.ts")?,
        expected(&[(
            "import",
            "Diamond",
            1,
            Some(("src/g.ts", "Diamond", "import_path"))
        )])
    );
    Ok(())
}

/// A file the index skips keeps its ties through imports, which need its imports: here `app.ts`
/// cannot be read on the second index, and its references keep their targets, an import
/// reference left unresolved included, though a definition of its name would tie it by name.
#[test]
fn a_skipped_file_keeps_its_ties_through_imports() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("src/order.ts", "export class Order {}\n"),
            ("src/hooks.ts", "export function useState() {}\n"),
            (
                "src/app.ts",
                "import { Order } from \"./order\";\nimport { useState } from \"react\";\n\
                 new Order();\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let order = Some(("src/order.ts", "Order", "import_path"));
    let tied = expected(&[
        ("import", "Order", 1, order),
        ("import", "useState", 2, None),
        ("call", "Order", 3, order),
    ]);
    assert_eq!(ties(root, "src/app.ts")?, tied);
    let locked = lock_out(&root.join("src/app.ts"))?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    drop(locked);
    // Unix leaves the file to be listed and refuses to open it; Windows refuses even to inspect
    // it, which the walk reports.
    let app = RepoPath::new("src/app.ts")?;
    let is_skipped = report.problems.iter().any(|(path, _)| *path == app)
        || report
            .walk_problems
            .iter()
            .any(|problem| problem.path.as_ref() == Some(&app));
    assert!(is_skipped, "app.ts was read");
    assert_eq!(ties(root, "src/app.ts")?, tied);
    Ok(())
}

/// A file that passes a name on must give one answer for it. Its own definition of the name and
/// its imports of the name must lead to the same definition: `shop` imports `loads` from `fast`,
/// outside the repository, or else from `.compat`, and `pkg` imports `dumps` from `fast` or else
/// defines it, so which holds depends on what runs and both stay unresolved. Its `*` imports
/// cannot stand in for an import of the name that finds nothing (TypeScript's explicit
/// `export { parse } from "lib"` wins over `export * from "./local"`), and they make the name
/// ambiguous when they find another definition (`later` defines `f`, then a later `*` import
/// brings in `a.f`, which wins in Python). Routes that agree tie: `kit` passes `Thing` on both by
/// name and with `*` from the one file that makes it.
#[test]
fn routes_through_a_file_must_agree() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (
                "shop/__init__.py",
                "try:\n    from fast import loads\nexcept ImportError:\n    \
                 from .compat import loads\n",
            ),
            ("shop/compat.py", "def loads(text):\n    return text\n"),
            (
                "pkg/__init__.py",
                "try:\n    from fast import dumps\nexcept ImportError:\n    \
                 def dumps(value):\n        return value\n",
            ),
            (
                "later/__init__.py",
                "def f():\n    pass\n\nfrom .a import *\n",
            ),
            ("later/a.py", "def f():\n    pass\n"),
            (
                "app.py",
                "from shop import loads\nfrom pkg import dumps\nfrom later import f\n\n\
                 loads(1)\ndumps(2)\nf()\n",
            ),
            (
                "src/lib/index.ts",
                "export { parse } from \"lib\";\nexport * from \"./local\";\n",
            ),
            ("src/lib/local.ts", "export function parse() {}\n"),
            (
                "src/kit/index.ts",
                "export { Thing } from \"./thing\";\nexport * from \"./thing\";\n",
            ),
            ("src/kit/thing.ts", "export class Thing {}\n"),
            (
                "src/app.ts",
                "import { parse } from \"./lib\";\nimport { Thing } from \"./kit\";\nparse();\n\
                 new Thing();\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "loads", 1, None),
            ("import", "dumps", 2, None),
            ("import", "f", 3, None),
            ("call", "loads", 5, None),
            ("call", "dumps", 6, None),
            ("call", "f", 7, None),
        ])
    );
    let thing = Some(("src/kit/thing.ts", "Thing", "import_path"));
    assert_eq!(
        ties(root, "src/app.ts")?,
        expected(&[
            ("import", "parse", 1, None),
            ("import", "Thing", 2, thing),
            ("call", "parse", 3, None),
            ("call", "Thing", 4, thing),
        ])
    );
    Ok(())
}
