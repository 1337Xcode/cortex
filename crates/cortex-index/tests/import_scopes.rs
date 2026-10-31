//! Where an import binds a name, after a full index of small repositories: only bare uses, only in
//! the scope the import is made in, only when every import binding the name there agrees, `*`
//! imports included; and how far a search for a name goes.

mod common;

use std::time::Duration;

use common::{TestResult, expected, index, lock_out, ties, write, write_all};
use cortex_core::RepoPath;

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// A name written after a qualifier is not the bare name an import binds. In Python
/// `line.strip()` is a method of `line`, not the imported `strip`, and goes by its name, here to
/// the only `strip` of the family; `strip(line)` is the import's. In TypeScript `items.map(f)`
/// and `map(items)` are told apart the same way. In Rust `Order::new()` is the type's method, not
/// the imported free function `new`, and by its name finds two definitions.
#[test]
fn qualified_uses_are_not_bound_by_imports() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("textutils.py", "def strip(text):\n    return text\n"),
            (
                "app.py",
                "from textutils import strip\n\ndef clean(line):\n    line.strip()\n    \
                 return strip(line)\n",
            ),
            ("src/util.ts", "export function map(xs) { return xs; }\n"),
            (
                "src/a.ts",
                "import { map } from \"./util\";\nconst items = [1];\nitems.map(f);\nmap(items);\n",
            ),
            ("Cargo.toml", "[package]\nname = \"shop\"\n"),
            (
                "src/lib.rs",
                "pub mod fixtures;\npub mod models;\npub mod api;\n",
            ),
            ("src/fixtures.rs", "pub fn new() -> u32 {\n    0\n}\n"),
            (
                "src/models.rs",
                "pub struct Order;\n\nimpl Order {\n    pub fn new() -> Self {\n        Order\n    }\n}\n",
            ),
            (
                "src/api.rs",
                "use crate::fixtures::new;\nuse crate::models::Order;\n\npub fn run() {\n    \
                 Order::new();\n    new();\n}\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let strip = ("textutils.py", "strip");
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            (
                "import",
                "strip",
                1,
                Some((strip.0, strip.1, "import_path"))
            ),
            ("call", "strip", 4, Some((strip.0, strip.1, "name_only"))),
            ("call", "strip", 5, Some((strip.0, strip.1, "import_path"))),
        ])
    );
    let map = ("src/util.ts", "map");
    assert_eq!(
        ties(root, "src/a.ts")?,
        expected(&[
            ("import", "map", 1, Some((map.0, map.1, "import_path"))),
            ("call", "map", 3, Some((map.0, map.1, "name_only"))),
            ("call", "map", 4, Some((map.0, map.1, "import_path"))),
        ])
    );
    let new = Some(("src/fixtures.rs", "new", "import_path"));
    assert_eq!(
        ties(root, "src/api.rs")?,
        expected(&[
            ("import", "new", 1, new),
            (
                "import",
                "Order",
                2,
                Some(("src/models.rs", "Order", "import_path"))
            ),
            ("call", "new", 5, None),
            ("call", "new", 6, new),
        ])
    );
    Ok(())
}

/// An import binds a name only in the definition it is made in and below, and a definition made
/// nearer the use shadows it. In Python, `create` imported in `g` is the import's in `g` and the
/// file's own `create` in `h`; `helper`, imported at the top, is the import's inside a method of a
/// class that has a method named `helper` too, since a class body is no scope for its methods,
/// but in `shadow.py` a function defining its own `helper` calls that one. In Rust, `build`
/// imported in `mod tests` is the import's in the tests and the file's own `build` elsewhere, and
/// in `user.rs` one imported inside `run` is the import's in `run` and the file's own in `other`.
#[test]
fn imports_bind_only_where_they_are_made() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("shop/__init__.py", ""),
            ("shop/other.py", "def create():\n    pass\n"),
            ("shop/models.py", "def helper():\n    pass\n"),
            (
                "shop/app.py",
                "from .models import helper\n\n\ndef create():\n    pass\n\n\ndef g():\n    \
                 from .other import create\n    return create()\n\n\ndef h():\n    \
                 return create()\n\n\nclass C:\n    def helper(self):\n        pass\n\n    \
                 def m(self):\n        return helper()\n",
            ),
            (
                "shop/shadow.py",
                "from .models import helper\n\n\ndef k():\n    def helper():\n        pass\n    \
                 return helper()\n",
            ),
            ("Cargo.toml", "[package]\nname = \"kit\"\n"),
            (
                "src/lib.rs",
                "pub mod fixtures;\n\nfn build() {}\n\nfn run() {\n    build();\n}\n\n\
                 #[cfg(test)]\nmod tests {\n    use crate::fixtures::build;\n\n    fn t() {\n        \
                 build();\n    }\n}\n",
            ),
            ("src/fixtures.rs", "pub fn build() {}\n"),
            (
                "src/user.rs",
                "fn build() {}\n\nfn run() {\n    use crate::fixtures::build;\n    build();\n}\n\n\
                 fn other() {\n    build();\n}\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let other = Some(("shop/other.py", "create", "import_path"));
    let helper = Some(("shop/models.py", "helper", "import_path"));
    assert_eq!(
        ties(root, "shop/app.py")?,
        expected(&[
            ("import", "helper", 1, helper),
            ("import", "create", 9, other),
            ("call", "create", 10, other),
            (
                "call",
                "create",
                14,
                Some(("shop/app.py", "create", "same_file"))
            ),
            ("call", "helper", 22, helper),
        ])
    );
    assert_eq!(
        ties(root, "shop/shadow.py")?,
        expected(&[
            ("import", "helper", 1, helper),
            (
                "call",
                "helper",
                7,
                Some(("shop/shadow.py", "k.helper", "same_file"))
            ),
        ])
    );
    let fixtures = Some(("src/fixtures.rs", "build", "import_path"));
    assert_eq!(
        ties(root, "src/lib.rs")?,
        expected(&[
            (
                "call",
                "build",
                6,
                Some(("src/lib.rs", "build", "same_file"))
            ),
            ("import", "build", 11, fixtures),
            ("call", "build", 14, fixtures),
        ])
    );
    assert_eq!(
        ties(root, "src/user.rs")?,
        expected(&[
            ("import", "build", 4, fixtures),
            ("call", "build", 5, fixtures),
            (
                "call",
                "build",
                9,
                Some(("src/user.rs", "build", "same_file"))
            ),
        ])
    );
    Ok(())
}

/// Every import that binds a name in one scope must find the same definition. A Python module
/// that imports `loads` from a package, else from its own `compat` module, cannot say which holds,
/// so the name stays unresolved, though the family has one `loads`; each import's own reference is
/// tied through that import alone. Two imports of `a` on one line stay apart: the one from a
/// module that does not make it finds nothing.
#[test]
fn every_import_binding_a_name_must_agree() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("shop/__init__.py", ""),
            ("shop/compat.py", "def loads(text):\n    return text\n"),
            (
                "shop/codec.py",
                "try:\n    from fast import loads\nexcept ImportError:\n    \
                 from .compat import loads\n\n\ndef decode(text):\n    return loads(text)\n",
            ),
            ("src/x.ts", "export const other = 1;\n"),
            ("src/y.ts", "export function a() {}\n"),
            (
                "src/one.ts",
                "import { a } from \"./x\"; import { a as b } from \"./y\";\nb();\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let compat = ("shop/compat.py", "loads");
    assert_eq!(
        ties(root, "shop/codec.py")?,
        expected(&[
            ("import", "loads", 2, None),
            (
                "import",
                "loads",
                4,
                Some((compat.0, compat.1, "import_path"))
            ),
            ("call", "loads", 8, None),
        ])
    );
    let a = Some(("src/y.ts", "a", "import_path"));
    assert_eq!(
        ties(root, "src/one.ts")?,
        expected(&[
            ("import", "a", 1, None),
            ("import", "a", 1, a),
            ("call", "b", 2, a),
        ])
    );
    Ok(())
}

/// An import of every name binds no bare name, since which names it brings in depends on each
/// language's rules of visibility and order: Python's `from .core import *` leaves `total` to the
/// name rules, which find two, and Go's dot import leaves `Place` to the family's one `Place`. An
/// import that takes the name binds it: in `both.py`, `total` is `other.py`'s. A TypeScript
/// re-export binds nothing in its own file either: the barrel's `Request` goes by its name.
#[test]
fn imports_of_every_name_and_reexports_bind_no_bare_name() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("shop/__init__.py", ""),
            ("shop/core.py", "def total():\n    return 0\n"),
            ("shop/other.py", "def total():\n    return 1\n"),
            (
                "shop/report.py",
                "from .core import *\n\n\ndef show():\n    return total()\n",
            ),
            (
                "shop/both.py",
                "from .core import *\nfrom .other import total\n\n\ndef show():\n    \
                 return total()\n",
            ),
            ("go.mod", "module example.com/shop\n"),
            ("orders/orders.go", "package orders\n\nfunc Place() {}\n"),
            (
                "cmd/main.go",
                "package main\n\nimport . \"example.com/shop/orders\"\n\nfunc main() {\n\tPlace()\n}\n",
            ),
            ("src/types.ts", "export interface Request {}\n"),
            (
                "src/index.ts",
                "export { Request } from \"./types\";\nfunction f(r: Request) {}\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "shop/report.py")?,
        expected(&[("call", "total", 5, None)])
    );
    let other = Some(("shop/other.py", "total", "import_path"));
    assert_eq!(
        ties(root, "shop/both.py")?,
        expected(&[("import", "total", 2, other), ("call", "total", 6, other)])
    );
    assert_eq!(
        ties(root, "cmd/main.go")?,
        expected(&[(
            "call",
            "Place",
            6,
            Some(("orders/orders.go", "Place", "name_only"))
        )])
    );
    let request = ("src/types.ts", "Request");
    assert_eq!(
        ties(root, "src/index.ts")?,
        expected(&[
            (
                "import",
                "Request",
                1,
                Some((request.0, request.1, "import_path"))
            ),
            (
                "type_ref",
                "Request",
                2,
                Some((request.0, request.1, "name_only"))
            ),
        ])
    );
    Ok(())
}

/// A namespace is not followed: `export * as ns` passes on the name `ns`, not the names of its
/// module. One search looks into at most 256 files, and finds a name only when it finishes within
/// them: `Edge`, made in the first of the files an index file brings in with `*`, is found behind
/// one of 255 such lines, where the search takes 256 looks, and not behind one of 256.
#[test]
fn searches_follow_names_only_and_only_so_far() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    let mut files = vec![
        ("src/x.ts".to_owned(), "export class Thing {}\n".to_owned()),
        (
            "src/barrel.ts".to_owned(),
            "export * as ns from \"./x\";\n".to_owned(),
        ),
        (
            "src/use.ts".to_owned(),
            "import { Thing } from \"./barrel\";\nimport { Edge } from \"./fits\";\n\
             import { Edge } from \"./over\";\n"
                .to_owned(),
        ),
    ];
    let mut fits = String::new();
    let mut over = String::new();
    for module in 0..256 {
        let line = format!("export * from \"./m{module:03}\";\n");
        if module < 255 {
            fits.push_str(&line);
        }
        over.push_str(&line);
        let text = if module == 0 {
            "export class Edge {}\n"
        } else {
            "export const v = 0;\n"
        };
        files.push((format!("src/m{module:03}.ts"), text.to_owned()));
    }
    files.push(("src/fits.ts".to_owned(), fits));
    files.push(("src/over.ts".to_owned(), over));
    for (path, text) in &files {
        write(root, path, text.as_bytes())?;
    }
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "src/use.ts")?,
        expected(&[
            ("import", "Thing", 1, None),
            (
                "import",
                "Edge",
                2,
                Some(("src/m000.ts", "Edge", "import_path"))
            ),
            ("import", "Edge", 3, None),
        ])
    );
    Ok(())
}

/// The same files give the same ties on one thread and on four, over more than one batch of
/// files: 300 TypeScript files take `Order` through an index file, beside a Python package that
/// passes `Order` on and a function that imports it for itself.
#[test]
fn thread_counts_change_no_tie() -> TestResult {
    let mut files: Vec<(String, String)> = [
        ("shop/__init__.py", "from .models import Order\n"),
        ("shop/models.py", "class Order:\n    pass\n"),
        (
            "shop/app.py",
            "from shop import Order\n\nOrder()\n\n\ndef make():\n    \
             from shop.models import Order\n    return Order()\n",
        ),
        ("src/order.ts", "export class Order {}\n"),
        ("src/index.ts", "export { Order } from \"./order\";\n"),
    ]
    .into_iter()
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .collect();
    for user in 0..300 {
        let text = "import { Order } from \"./index\";\nnew Order();\n".to_owned();
        files.push((format!("src/use{user:03}.ts"), text));
    }
    let mut found = Vec::new();
    for threads in [1, 4] {
        let repo = tempfile::tempdir()?;
        for (path, text) in &files {
            write(repo.path(), path, text.as_bytes())?;
        }
        index(repo.path(), threads, GENEROUS, DEFAULT_LIMIT)?;
        let mut all = Vec::new();
        for (path, _) in &files {
            all.push(ties(repo.path(), path)?);
        }
        found.push(all);
    }
    assert_eq!(found[0], found[1]);
    let python = Some(("shop/models.py", "Order", "import_path"));
    assert_eq!(
        found[0][2],
        expected(&[
            ("import", "Order", 1, python),
            ("call", "Order", 3, python),
            ("import", "Order", 7, python),
            ("call", "Order", 8, python),
        ])
    );
    let typescript = Some(("src/order.ts", "Order", "import_path"));
    assert_eq!(
        found[0][304],
        expected(&[
            ("import", "Order", 1, typescript),
            ("call", "Order", 2, typescript)
        ])
    );
    Ok(())
}

/// A file the index skips keeps its ties through imports, but its other references go by name
/// again: once a second `helper` appears elsewhere, the skipped file's `helper()` is ambiguous.
#[test]
fn a_skipped_file_s_other_references_go_by_name_again() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("src/order.ts", "export class Order {}\n"),
            ("src/hooks.ts", "export function helper() {}\n"),
            (
                "src/app.ts",
                "import { Order } from \"./order\";\nnew Order();\nhelper();\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let order = Some(("src/order.ts", "Order", "import_path"));
    let helper = Some(("src/hooks.ts", "helper", "name_only"));
    assert_eq!(
        ties(root, "src/app.ts")?,
        expected(&[
            ("import", "Order", 1, order),
            ("call", "Order", 2, order),
            ("call", "helper", 3, helper),
        ])
    );
    write(root, "src/more.ts", b"export function helper() {}\n")?;
    let locked = lock_out(&root.join("src/app.ts"))?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    drop(locked);
    let app = RepoPath::new("src/app.ts")?;
    let is_skipped = report.problems.iter().any(|(path, _)| *path == app)
        || report
            .walk_problems
            .iter()
            .any(|problem| problem.path.as_ref() == Some(&app));
    assert!(is_skipped, "app.ts was read");
    assert_eq!(
        ties(root, "src/app.ts")?,
        expected(&[
            ("import", "Order", 1, order),
            ("call", "Order", 2, order),
            ("call", "helper", 3, None),
        ])
    );
    Ok(())
}

/// A Rust module sees nothing of the scopes around it, and `super` inside an inline module starts
/// from that module. In `src/a.rs`, `helper()` in `top` is bound by the file's `use` of
/// `util::helper`; in `mod inner`, which brings in everything of `other`, the file's `use` does
/// not reach, and `helper()` goes by its name, finding three. In `mod tests`, `super::helper` is
/// the file's own `helper`, the one it brings in from `util`, and `super::super::helper` the
/// crate root's.
#[test]
fn rust_modules_see_nothing_around_them() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("Cargo.toml", "[package]\nname = \"shop\"\n"),
            (
                "src/lib.rs",
                "pub mod a;\npub mod util;\npub mod other;\n\npub fn helper() {}\n",
            ),
            ("src/util.rs", "pub fn helper() {}\n"),
            ("src/other.rs", "pub fn helper() {}\n"),
            (
                "src/a.rs",
                "use crate::util::helper;\n\npub fn top() {\n    helper();\n}\n\nmod inner {\n    \
                 use crate::other::*;\n\n    pub fn run() {\n        helper();\n    }\n}\n\n\
                 mod tests {\n    use super::helper;\n    use super::super::helper as root_helper;\n\n    \
                 fn t() {\n        helper();\n        root_helper();\n    }\n}\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let util = Some(("src/util.rs", "helper", "import_path"));
    let crate_root = Some(("src/lib.rs", "helper", "import_path"));
    assert_eq!(
        ties(root, "src/a.rs")?,
        expected(&[
            ("import", "helper", 1, util),
            ("call", "helper", 4, util),
            ("call", "helper", 11, None),
            ("import", "helper", 16, util),
            ("import", "helper", 17, crate_root),
            ("call", "helper", 20, util),
            ("call", "root_helper", 21, crate_root),
        ])
    );
    Ok(())
}
