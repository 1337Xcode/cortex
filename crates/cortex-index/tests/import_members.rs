//! Names written after a qualifier that an import gives a module, after a full index of small
//! repositories: tied to that module's definition, or left unresolved when the module is outside
//! the repository; a qualifier an import gives a definition leaves the name to its own name.

mod common;

use std::time::Duration;

use common::{TestResult, expected, index, ties, write_all};

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// Go: a package is reached by its path's last element (`orders`), or the one before a major
/// version (`example.com/shop/v2` is `shop`), or its alias (`h`). `orders.Place()` and
/// `orders.Order` are the package's; `shop.Open()` the v2 module's. `h.Error()` and
/// `yaml.Unmarshal()` name packages outside the repository and stay unresolved, though the
/// repository has an `Error` and an `Unmarshal`. `o.Total()` is a method on a variable and goes by
/// its name, to the one `Total`; `util.Error()` names a package imported only under the blank
/// name, which binds nothing, so it too goes by its name.
#[test]
fn go_names_after_a_package_are_the_package_s() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("go.mod", "module example.com/shop\n"),
            (
                "orders/orders.go",
                "package orders\n\nfunc Place() {}\n\ntype Order struct{}\n",
            ),
            (
                "orders/total.go",
                "package orders\n\nfunc (o Order) Total() int { return 0 }\n",
            ),
            (
                "util/util.go",
                "package util\n\nfunc Error() {}\n\nfunc Unmarshal() {}\n",
            ),
            ("v2/go.mod", "module example.com/shop/v2\n"),
            ("v2/shop.go", "package shop\n\nfunc Open() {}\n"),
            (
                "main.go",
                "package main\n\nimport (\n\t\"example.com/shop/orders\"\n\th \"net/http\"\n\t\
                 \"example.com/shop/v2\"\n\t\"gopkg.in/yaml.v3\"\n\t_ \"example.com/shop/util\"\n)\n\n\
                 func main() {\n\torders.Place()\n\tvar o orders.Order\n\to.Total()\n\th.Error()\n\t\
                 shop.Open()\n\tyaml.Unmarshal()\n\tutil.Error()\n}\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "main.go")?,
        expected(&[
            (
                "call",
                "Place",
                12,
                Some(("orders/orders.go", "Place", "import_path"))
            ),
            (
                "type_ref",
                "Order",
                13,
                Some(("orders/orders.go", "Order", "import_path"))
            ),
            (
                "call",
                "Total",
                14,
                Some(("orders/total.go", "Order.Total", "name_only"))
            ),
            ("call", "Error", 15, None),
            (
                "call",
                "Open",
                16,
                Some(("v2/shop.go", "Open", "import_path"))
            ),
            ("call", "Unmarshal", 17, None),
            (
                "call",
                "Error",
                18,
                Some(("util/util.go", "Error", "name_only"))
            ),
        ])
    );
    Ok(())
}

/// TypeScript: a namespace import reaches its module and what the module passes on, by name
/// (`fetchOrders`, though two files define it) or with `*` (`Order`). A namespace of a package
/// outside the repository (`fs`) leaves `readFile` unresolved. `Client.connect()` is a member of
/// the default export, a class, and goes by its name; `items.fetchOrders()` goes by its name too
/// and finds two.
#[test]
fn typescript_names_after_a_namespace_are_its_module_s() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (
                "src/api/index.ts",
                "export { fetchOrders } from \"./fetch\";\nexport * from \"./types\";\n",
            ),
            ("src/api/fetch.ts", "export function fetchOrders() {}\n"),
            ("src/api/types.ts", "export interface Order {}\n"),
            ("src/local.ts", "export function fetchOrders() {}\n"),
            (
                "src/client.ts",
                "export default class Client {\n  static connect() {}\n}\n",
            ),
            (
                "src/app.ts",
                "import * as api from \"./api\";\nimport * as fs from \"fs\";\n\
                 import Client from \"./client\";\napi.fetchOrders();\nlet o: api.Order;\n\
                 fs.readFile();\nClient.connect();\nitems.fetchOrders();\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "src/app.ts")?,
        expected(&[
            (
                "call",
                "fetchOrders",
                4,
                Some(("src/api/fetch.ts", "fetchOrders", "import_path"))
            ),
            (
                "type_ref",
                "Order",
                5,
                Some(("src/api/types.ts", "Order", "import_path"))
            ),
            ("call", "readFile", 6, None),
            (
                "call",
                "connect",
                7,
                Some(("src/client.ts", "Client.connect", "name_only"))
            ),
            ("call", "fetchOrders", 8, None),
        ])
    );
    Ok(())
}

/// Python: `import shop.models` reaches the module as `shop.models`, `as m` as `m`, and
/// `from shop import models` as `models`, the submodule, since `shop/__init__.py` defines no
/// `models`. `from pkg import tools` takes the function `tools` that `pkg/__init__.py` defines, not
/// the submodule `pkg/tools.py`, so `tools.run()` goes by its name and finds two. `os.path` is
/// outside the repository and leaves `join` unresolved. An import made in a function reaches its
/// module there only: `n.helper()` at the top goes by its name.
#[test]
fn python_names_after_a_module_are_the_module_s() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("shop/__init__.py", ""),
            (
                "shop/models.py",
                "class Order:\n    pass\n\ndef helper():\n    pass\n",
            ),
            ("pkg/__init__.py", "def tools():\n    pass\n"),
            ("pkg/tools.py", "def run():\n    pass\n"),
            (
                "elsewhere.py",
                "def run():\n    pass\n\ndef join():\n    pass\n",
            ),
            (
                "app.py",
                "import shop.models\nimport shop.models as m\nfrom shop import models\n\
                 from pkg import tools\nimport os.path\n\n\ndef main():\n    shop.models.Order()\n    \
                 m.helper()\n    models.Order()\n    tools.run()\n    os.path.join()\n\n\n\
                 def other():\n    import shop.models as n\n    n.helper()\n\n\nn.helper()\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let models = "shop/models.py";
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "models", 3, None),
            (
                "import",
                "tools",
                4,
                Some(("pkg/__init__.py", "tools", "import_path"))
            ),
            ("call", "Order", 9, Some((models, "Order", "import_path"))),
            (
                "call",
                "helper",
                10,
                Some((models, "helper", "import_path"))
            ),
            ("call", "Order", 11, Some((models, "Order", "import_path"))),
            ("call", "run", 12, None),
            ("call", "join", 13, None),
            (
                "call",
                "helper",
                18,
                Some((models, "helper", "import_path"))
            ),
            ("call", "helper", 21, Some((models, "helper", "name_only"))),
        ])
    );
    Ok(())
}

/// Rust: `use crate::models;` and `use crate::models as m;` reach the module file of `models`,
/// declared by `pub mod models;` in the crate root, so `models::helper()` and `m::helper()` are
/// that file's. `use std::fs;` is outside the repository: `fs::read()` stays unresolved, though
/// the crate has a `read`. The imports' own references are tied to the declaration of `models`.
#[test]
fn rust_names_after_a_module_are_the_module_s() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("Cargo.toml", "[package]\nname = \"shop\"\n"),
            ("src/lib.rs", "pub mod models;\npub mod app;\npub mod io;\n"),
            ("src/models.rs", "pub fn helper() {}\n\npub struct Order;\n"),
            ("src/io.rs", "pub fn read() {}\n"),
            (
                "src/app.rs",
                "use crate::models;\nuse crate::models as m;\nuse std::fs;\n\npub fn run() {\n    \
                 models::helper();\n    m::helper();\n    fs::read();\n}\n",
            ),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let declared = Some(("src/lib.rs", "models", "import_path"));
    let helper = Some(("src/models.rs", "helper", "import_path"));
    assert_eq!(
        ties(root, "src/app.rs")?,
        expected(&[
            ("import", "models", 1, declared),
            ("import", "models", 2, declared),
            ("import", "fs", 3, None),
            ("call", "helper", 6, helper),
            ("call", "helper", 7, helper),
            ("call", "read", 8, None),
        ])
    );
    Ok(())
}
