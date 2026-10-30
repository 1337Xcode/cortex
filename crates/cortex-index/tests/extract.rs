//! Reading files: definitions, references and imports compared with expectations written by hand
//! from fixtures, then the hashes, test conventions, the limit on qualified names, broken input
//! and the time limit.

use std::error::Error;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cortex_core::{Language, RefKind, RepoPath, ShortHash, SymbolKind};
use cortex_index::{ExtractError, Extraction, Extractor, Grammars, Imported};

/// What the tests return: any failure, with its cause.
type TestResult = Result<(), Box<dyn Error>>;

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// Every grammar, loaded once for all the tests here: compiling the queries takes longer than
/// anything a test reads.
fn grammars() -> Result<Arc<Grammars>, Box<dyn Error>> {
    static GRAMMARS: OnceLock<Result<Arc<Grammars>, String>> = OnceLock::new();
    GRAMMARS
        .get_or_init(|| {
            Grammars::load()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(Into::into)
}

/// An extractor over every grammar, allowing each file `timeout`.
fn extractor(timeout: Duration) -> Result<Extractor, Box<dyn Error>> {
    Ok(Extractor::new(grammars()?, timeout))
}

/// What `source` gives when read as the file at `path`, written in `language`.
fn extract(path: &str, language: Language, source: &str) -> Result<Extraction, Box<dyn Error>> {
    Ok(extractor(GENEROUS)?.extract(&RepoPath::new(path)?, language, source.as_bytes())?)
}

/// `text` with each run of ASCII whitespace as one space and none at either end, written
/// independently of the crate's own.
fn collapsed(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// The 1-based line of `source` that holds the byte at `offset`.
fn line_at(source: &str, offset: usize) -> Option<usize> {
    Some(source.get(..offset)?.matches('\n').count() + 1)
}

/// The extraction in the format of the `expected.txt` files, without the headers, which a
/// record holds only as a hash.
fn describe(extraction: &Extraction) -> Vec<String> {
    let enclosing = |position: Option<usize>| {
        position
            .and_then(|position| extraction.symbols.get(position))
            .map_or("-", |symbol| symbol.qual_name.as_str())
    };
    let symbols = extraction.symbols.iter().map(|symbol| {
        format!(
            "symbol {} {} {}-{} {}",
            symbol.kind.name(),
            symbol.qual_name.as_str(),
            symbol.start_line,
            symbol.end_line,
            enclosing(symbol.parent)
        )
    });
    let refs = extraction.refs.iter().map(|reference| {
        format!(
            "ref {} {} {} {}",
            reference.kind.name(),
            reference.dst_name.as_str(),
            reference.line,
            enclosing(reference.symbol)
        )
    });
    let imports = extraction.imports.iter().map(|import| {
        let imported = match &import.imported {
            Imported::Module => "module".to_owned(),
            Imported::Name(name) => format!("name:{}", name.as_str()),
            Imported::All => "all".to_owned(),
            Imported::Default => "default".to_owned(),
        };
        let alias = import.alias.as_ref().map_or("-", |alias| alias.as_str());
        format!(
            "import {} {imported} {alias} {}",
            import.specifier.as_str(),
            import.line
        )
    });
    symbols.chain(refs).chain(imports).collect()
}

/// Reads the fixture `tests/languages/<folder>/<sample>`, with its line breaks written as
/// `line_end`, as a file in `language`, and checks it against `expected.txt` beside it: every
/// line, then each symbol's span and hashes against the header written there.
fn check_golden(folder: &str, sample: &str, language: Language, line_end: &str) -> TestResult {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/languages")
        .join(folder);
    let source = std::fs::read_to_string(folder.join(sample))?.replace('\n', line_end);
    let expected = std::fs::read_to_string(folder.join("expected.txt"))?;
    let expected: Vec<&str> = expected
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let extraction = extract(&format!("shop/{sample}"), language, &source)?;
    let lines: Vec<&str> = expected
        .iter()
        .map(|line| line.split(" | ").next().unwrap_or_default())
        .collect();
    assert_eq!(describe(&extraction), lines);
    let headers = expected.iter().filter_map(|line| line.split_once(" | "));
    for (symbol, (_, header)) in extraction.symbols.iter().zip(headers) {
        let name = symbol.qual_name.as_str();
        let start = usize::try_from(symbol.start_byte)?;
        let end = usize::try_from(symbol.end_byte)?;
        let text = source.get(start..end).ok_or("span outside the file")?;
        assert_eq!(
            line_at(&source, start),
            Some(symbol.start_line.try_into()?),
            "{name}"
        );
        assert_eq!(
            line_at(&source, end - 1),
            Some(symbol.end_line.try_into()?),
            "{name}"
        );
        assert!(collapsed(text).starts_with(header), "{name}");
        assert!(!text.ends_with(char::is_whitespace), "{name}");
        assert_eq!(symbol.sig_hash, ShortHash::of(header.as_bytes()), "{name}");
        assert_eq!(
            symbol.body_hash,
            ShortHash::of(collapsed(text).as_bytes()),
            "{name}"
        );
    }
    Ok(())
}

/// Python: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn python() -> TestResult {
    check_golden("python", "sample.py", Language::Python, "\n")
}

/// JavaScript: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn javascript() -> TestResult {
    check_golden("javascript", "sample.js", Language::JavaScript, "\n")
}

/// TypeScript: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn typescript() -> TestResult {
    check_golden("typescript", "sample.ts", Language::TypeScript, "\n")
}

/// TSX: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn tsx() -> TestResult {
    check_golden("tsx", "sample.tsx", Language::Tsx, "\n")
}

/// Go: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn go() -> TestResult {
    check_golden("go", "sample.go", Language::Go, "\n")
}

/// Rust: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn rust() -> TestResult {
    check_golden("rust", "sample.rs", Language::Rust, "\n")
}

/// Java: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn java() -> TestResult {
    check_golden("java", "sample.java", Language::Java, "\n")
}

/// C: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn c() -> TestResult {
    check_golden("c", "sample.c", Language::C, "\n")
}

/// The same C with Windows line breaks, which end its `#define` lines: the same lines, spans
/// that end at the same tokens, the same hashes.
#[test]
fn c_with_crlf_line_breaks() -> TestResult {
    check_golden("c", "sample.c", Language::C, "\r\n")
}

/// C++: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn cpp() -> TestResult {
    check_golden("cpp", "sample.cpp", Language::Cpp, "\n")
}

/// The same C++ with Windows line breaks: the same lines, spans that end at the same tokens,
/// the same hashes.
#[test]
fn cpp_with_crlf_line_breaks() -> TestResult {
    check_golden("cpp", "sample.cpp", Language::Cpp, "\r\n")
}

/// C#: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn csharp() -> TestResult {
    check_golden("csharp", "sample.cs", Language::CSharp, "\n")
}

/// PHP: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn php() -> TestResult {
    check_golden("php", "sample.php", Language::Php, "\n")
}

/// Kotlin: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn kotlin() -> TestResult {
    check_golden("kotlin", "sample.kt", Language::Kotlin, "\n")
}

/// Ruby: every construct in the fixture, listed by hand in `expected.txt`.
#[test]
fn ruby() -> TestResult {
    check_golden("ruby", "sample.rb", Language::Ruby, "\n")
}

/// The same Python with Windows line breaks: the same lines, spans that end at the same tokens,
/// the same hashes.
#[test]
fn python_with_crlf_line_breaks() -> TestResult {
    check_golden("python", "sample.py", Language::Python, "\r\n")
}

/// A name is the name alone, even for a definition that fits on one line.
#[test]
fn names_are_never_whole_definitions() -> TestResult {
    let extraction = extract(
        "shop/short.py",
        Language::Python,
        "def f(): pass\nclass C: pass\nX = 1\n",
    )?;
    let names: Vec<&str> = extraction
        .symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(names, ["f", "C", "X"]);
    Ok(())
}

/// Rewrapping and reindenting keep both hashes; a new body changes only the body's hash; a new
/// header changes both. Comments between the header and the body are not part of the header,
/// and an assignment's value is its body.
#[test]
fn hashes_follow_the_header_and_the_body() -> TestResult {
    let hashes = |source: &str| -> Result<(ShortHash, ShortHash), Box<dyn Error>> {
        let extraction = extract("shop/area.py", Language::Python, source)?;
        let symbol = extraction.symbols.first().ok_or("no symbol")?;
        Ok((symbol.sig_hash, symbol.body_hash))
    };
    let original = hashes("def area(width, height):\n    return width * height\n")?;
    assert_eq!(
        original,
        (
            ShortHash::of(b"def area(width, height):"),
            ShortHash::of(b"def area(width, height): return width * height")
        )
    );
    assert_eq!(
        hashes("def area(width,  height):\n\treturn width   *   height\n")?,
        original
    );
    let commented = hashes(
        "def area(width, height):  # of a box\n    # width times height\n    return width * height\n",
    )?;
    assert_eq!(commented.0, original.0);
    let new_body = hashes("def area(width, height):\n    return width * height * 1\n")?;
    assert_eq!(new_body.0, original.0);
    assert_ne!(new_body.1, original.1);
    let new_header = hashes("def area(width, height=1):\n    return width * height\n")?;
    assert_ne!(new_header.0, original.0);
    assert_ne!(new_header.1, original.1);
    let one = hashes("LIMIT = 1\n")?;
    let two = hashes("LIMIT = 2\n")?;
    assert_eq!(
        (one.0, two.0),
        (ShortHash::of(b"LIMIT ="), ShortHash::of(b"LIMIT ="))
    );
    assert_eq!(one.1, ShortHash::of(b"LIMIT = 1"));
    assert_ne!(one.1, two.1);
    Ok(())
}

/// In a test file, functions and methods named the way pytest names tests are tests; the same
/// code elsewhere has none.
#[test]
fn tests_are_marked_only_in_test_files() -> TestResult {
    use SymbolKind::{Class, Function, Method, Test};
    let source = "class TestOrder:\n    def test_total(self):\n        assert total([]) == 0\n\n    \
                  def helper(self):\n        pass\n\n\ndef test_ship():\n    ship(None)\n\n\n\
                  def make_order():\n    pass\n";
    let kinds = |path: &str| -> Result<Vec<(SymbolKind, String)>, Box<dyn Error>> {
        let extraction = extract(path, Language::Python, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| (symbol.kind, symbol.qual_name.as_str().to_owned()))
            .collect())
    };
    let named = |kinds: [SymbolKind; 5]| -> Vec<(SymbolKind, String)> {
        let names = [
            "TestOrder",
            "TestOrder.test_total",
            "TestOrder.helper",
            "test_ship",
            "make_order",
        ];
        kinds
            .into_iter()
            .zip(names)
            .map(|(kind, name)| (kind, name.to_owned()))
            .collect()
    };
    for path in [
        "tests/orders.py",
        "shop/test_orders.py",
        "shop/orders_test.py",
    ] {
        assert_eq!(
            kinds(path)?,
            named([Class, Test, Method, Test, Function]),
            "{path}"
        );
    }
    assert_eq!(
        kinds("shop/orders.py")?,
        named([Class, Method, Method, Function, Function])
    );
    Ok(())
}

/// In a Go test file, the functions `go test` runs (tests, benchmarks, examples and fuzz targets)
/// are tests; the same code elsewhere has none.
#[test]
fn go_tests_are_marked_only_in_test_files() -> TestResult {
    use SymbolKind::{Function, Test};
    let source = "package shop\n\nfunc TestPay(t *testing.T) {}\nfunc BenchmarkPay(b *testing.B) {}\n\
                  func ExamplePay() {}\nfunc FuzzPay(f *testing.F) {}\nfunc helper() {}\n";
    let kinds = |path: &str| -> Result<Vec<SymbolKind>, Box<dyn Error>> {
        let extraction = extract(path, Language::Go, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| symbol.kind)
            .collect())
    };
    assert_eq!(
        kinds("shop/pay_test.go")?,
        [Test, Test, Test, Test, Function]
    );
    assert_eq!(kinds("shop/pay.go")?, [Function; 5]);
    Ok(())
}

/// In Rust a test is a function with a test attribute, `#[test]` or a crate's such as
/// `#[tokio::test]`, other attributes between allowed, wherever the file is.
#[test]
fn rust_tests_are_marked_by_attribute() -> TestResult {
    use SymbolKind::{Function, Test};
    let source = "#[test]\nfn a() {}\n#[tokio::test]\nasync fn b() {}\n#[test]\n#[ignore]\nfn c() {}\n\
                  #[inline]\nfn d() {}\n#[tracing::instrument]\nfn e() {}\n";
    let extraction = extract("src/orders.rs", Language::Rust, source)?;
    let kinds: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.name.as_str(), symbol.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("a", Test),
            ("b", Test),
            ("c", Test),
            ("d", Function),
            ("e", Function)
        ]
    );
    Ok(())
}

/// Methods and constants in a Rust `impl` are named with its type, plain, generic or behind a
/// path, for a trait's impl as for an inherent one.
#[test]
fn rust_impls_name_their_items() -> TestResult {
    let source = "impl<T> Stack<T> {\n    const N: u32 = 1;\n    fn push(&mut self) {}\n}\n\
                  impl fmt::Display for Wrapper {\n    fn fmt(&self) {}\n}\n\
                  impl geo::Point {\n    fn x(&self) {}\n}\n";
    let extraction = extract("src/shapes.rs", Language::Rust, source)?;
    let names: Vec<&str> = extraction
        .symbols
        .iter()
        .map(|symbol| symbol.qual_name.as_str())
        .collect();
    assert_eq!(names, ["Stack.N", "Stack.push", "Wrapper.fmt", "Point.x"]);
    Ok(())
}

/// In Java a method annotated as a test is a test wherever the file is, by plain or qualified
/// name; JUnit 3's `test*` methods are tests only in a test file.
#[test]
fn java_tests_are_marked_by_annotation_or_name() -> TestResult {
    use SymbolKind::{Class, Method, Test};
    let source = "class OrderTest {\n    @org.junit.jupiter.api.Test void a() {}\n    \
                  @ParameterizedTest @ValueSource(ints = {1}) void b(int x) {}\n    \
                  @Disabled void c() {}\n    void testD() {}\n    void helper() {}\n    \
                  @Test(timeout = 100) void e() {}\n    @lombok.Generated void f() {}\n}\n";
    let kinds = |path: &str| -> Result<Vec<SymbolKind>, Box<dyn Error>> {
        let extraction = extract(path, Language::Java, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| symbol.kind)
            .collect())
    };
    assert_eq!(
        kinds("shop/OrderTest.java")?,
        [Class, Test, Test, Method, Test, Method, Test, Method]
    );
    assert_eq!(
        kinds("shop/Order.java")?,
        [Class, Test, Test, Method, Method, Method, Test, Method]
    );
    Ok(())
}

/// In a Ruby test file, the methods Minitest runs, named `test_`, are tests, and so is such a
/// function; the same code elsewhere has none. `testing` lacks the underscore.
#[test]
fn ruby_tests_are_marked_only_in_test_files() -> TestResult {
    use SymbolKind::{Class, Function, Method, Test};
    let source = "class OrderTest < Minitest::Test\n  def test_total\n    assert_equal 0, total\n  end\n\n  \
                  def helper\n  end\n\n  def testing\n  end\nend\n\ndef test_ship\nend\n";
    let kinds = |path: &str| -> Result<Vec<SymbolKind>, Box<dyn Error>> {
        let extraction = extract(path, Language::Ruby, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| symbol.kind)
            .collect())
    };
    for path in ["lib/order_test.rb", "lib/order_spec.rb", "test/order.rb"] {
        assert_eq!(kinds(path)?, [Class, Test, Method, Method, Test], "{path}");
    }
    assert_eq!(
        kinds("lib/order.rb")?,
        [Class, Method, Method, Method, Function]
    );
    Ok(())
}

/// Ruby base classes and the modules mixed into a class are inheritance, plain or qualified, one
/// or several at a time. `Order.include(Billing)` mixes into another object, not the class around
/// it, so `Billing` stays a use of the constant, as `Records` and `Audit` are.
#[test]
fn ruby_bases_and_mixins_are_inherited() -> TestResult {
    let source = "class Order < Records::Base\n  include Comparable, Audit::Trail\n  \
                  prepend Logging\n  Order.include(Billing)\nend\n";
    let extraction = extract("lib/order.rb", Language::Ruby, source)?;
    let refs: Vec<(&str, &str)> = extraction
        .refs
        .iter()
        .map(|reference| (reference.kind.name(), reference.dst_name.as_str()))
        .collect();
    assert_eq!(
        refs,
        [
            ("call", "Records"),
            ("inherit", "Base"),
            ("call", "include"),
            ("inherit", "Comparable"),
            ("call", "Audit"),
            ("inherit", "Trail"),
            ("call", "prepend"),
            ("inherit", "Logging"),
            ("call", "Order"),
            ("call", "include"),
            ("call", "Billing"),
        ]
    );
    Ok(())
}

/// Ruby constants are assigned with `=` or `||=`; several at once, or one inside another
/// namespace, are not read as definitions, and neither is a local variable.
#[test]
fn ruby_constants_are_assigned() -> TestResult {
    let source = "LIMIT = 3\nRATE ||= 2\nA, B = 1, 2\nShop::MAX = 4\nlimit = 5\n";
    let extraction = extract("lib/limits.rb", Language::Ruby, source)?;
    let symbols: Vec<(&str, SymbolKind, u32)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind, symbol.start_line))
        .collect();
    assert_eq!(
        symbols,
        [
            ("LIMIT", SymbolKind::Constant, 1),
            ("RATE", SymbolKind::Constant, 2)
        ]
    );
    Ok(())
}

/// A Ruby method or alias named by a symbol or a variable is not a definition: its name would
/// carry the sigil, and an interpolated symbol holds code. The grammar parses such methods
/// without an error. Only `cost` is read.
#[test]
fn ruby_names_written_as_symbols_are_left_out() -> TestResult {
    let source = "class A\n  alias cost total\n  alias :price :total\n  \
                  alias :\"x#{run(1)}\" y\n  alias $new $old\n  def :total; end\n  \
                  def @size; end\n  def self.:make; end\nend\ndef :\"a#{b}\"; end\n";
    let extraction = extract("lib/a.rb", Language::Ruby, source)?;
    let symbols: Vec<&str> = extraction
        .symbols
        .iter()
        .map(|symbol| symbol.qual_name.as_str())
        .collect();
    assert_eq!(symbols, ["A", "A.cost"]);
    Ok(())
}

/// A Ruby class, module or `class << self` body, and a method defined on an object, starts with
/// no local variables. So `size` on line 7 is a call, not the parameter of `make` (which it is on
/// line 5); `x` on lines 8 and 14 is a call, not the variable of line 1; and `y` on line 10 is a
/// call, not the variable of line 3.
#[test]
fn ruby_classes_and_object_methods_are_scopes() -> TestResult {
    let source = "x = 5\nclass A\n  y = 1\n  def self.make(size)\n    size\n  end\n  size\n  x\n  \
                  class << self\n    y\n  end\nend\nmodule K\n  x\nend\n";
    let extraction = extract("lib/a.rb", Language::Ruby, source)?;
    let refs: Vec<(&str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| (reference.dst_name.as_str(), reference.line))
        .collect();
    assert_eq!(refs, [("size", 7), ("x", 8), ("y", 10), ("x", 14)]);
    Ok(())
}

/// A Ruby file imports what `require`, `require_relative`, `load` and `autoload` load by a literal
/// path, in either kind of quotes and with other arguments after it. A path built at run time
/// (line 2) and `YAML.load`, a method of another object (line 3), import nothing.
#[test]
fn ruby_imports_need_a_literal_path() -> TestResult {
    let source = "require \"a\"\nrequire \"#{dir}/b\"\nYAML.load \"c.yml\"\nload \"d.rb\", true\n\
                  require_relative 'e'\nautoload :F, \"f\"\n";
    let extraction = extract("lib/order.rb", Language::Ruby, source)?;
    let imports: Vec<(&str, u32)> = extraction
        .imports
        .iter()
        .map(|import| (import.specifier.as_str(), import.line))
        .collect();
    assert_eq!(imports, [("a", 1), ("d.rb", 4), ("e", 5), ("f", 6)]);
    assert!(
        extraction
            .imports
            .iter()
            .all(|import| import.imported == Imported::Module && import.alias.is_none())
    );
    Ok(())
}

/// In a C test file, the functions named `test_` are tests; the same code elsewhere has none.
/// `testing` lacks the underscore, and the macro `test_assert` is a macro, not a test.
#[test]
fn c_tests_are_marked_only_in_test_files() -> TestResult {
    use SymbolKind::{Function, Test};
    let source = "#define test_assert(x) (x)\nint test_total(void) { return 0; }\n\
                  static int helper(void) { return 1; }\nint testing(void) { return 2; }\n";
    let kinds = |path: &str| -> Result<Vec<SymbolKind>, Box<dyn Error>> {
        let extraction = extract(path, Language::C, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| symbol.kind)
            .collect())
    };
    for path in ["tests/order.c", "src/test_order.c", "src/order_test.c"] {
        assert_eq!(kinds(path)?, [Function, Test, Function, Function], "{path}");
    }
    assert_eq!(kinds("src/order.c")?, [Function; 4]);
    Ok(())
}

/// C constants are variables that cannot change, declared outside any function: at the top of
/// the file, in an `#if`, `#ifndef` or `#else` block, or in an `extern "C"` block. A plain variable
/// or an array is `const` itself; a pointer has `const` after its star. A pointer to constant data
/// (`VAL`), a `volatile` variable, an include guard's `#define` and a `const` in a function or a
/// `for` are not constants.
#[test]
fn c_constants_are_variables_that_cannot_change() -> TestResult {
    let source = "static const char LABEL[] = \"x\";\n#if DEBUG\nconst int LEVEL = 2;\n#endif\n\
                  #ifndef GRID_H\n#define GRID_H\nstatic const int SIZE = 4;\n#else\n\
                  const int ELSE = 2;\n#endif\nextern \"C\" {\nconst int EXT = 1;\n}\n\
                  char *const PTR = 0;\nstatic const char *const NAMES[] = {\"a\", \"b\"};\n\
                  const char *VAL = 0;\nstatic volatile int flag = 0;\n\
                  int **grid(void) { const int n = 1; for (const int i = 0; i < n;) {} return 0; }\n";
    let extraction = extract("src/grid.c", Language::C, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Constant, Function};
    assert_eq!(
        symbols,
        [
            ("LABEL", Constant),
            ("LEVEL", Constant),
            ("SIZE", Constant),
            ("ELSE", Constant),
            ("EXT", Constant),
            ("PTR", Constant),
            ("NAMES", Constant),
            ("grid", Function)
        ]
    );
    Ok(())
}

/// Each name a C `typedef` gives is a type, and none sits inside another or inside the typedef
/// of another: one name or several, plain, pointer, array or enum, with attributes after it, or
/// beside a named struct the typedef defines. A name deeper in its declarator (`grid_t`) is not
/// read, and is not read as a use either; only its use on line 10 is, as a union and an enum
/// named without a body are uses on lines 11 and 12. The attribute's `aligned(8)` is not a
/// call.
#[test]
fn c_typedefs_name_each_type_once() -> TestResult {
    let source = "typedef unsigned long money;\ntypedef struct { int x; } point;\n\
                  typedef int *intptr;\ntypedef enum status { OPEN } status_t;\n\
                  typedef struct node { int v; } node_t, *node_ptr;\ntypedef int a, b;\n\
                  typedef int cint __attribute__((aligned(8)));\ntypedef char label[32];\n\
                  typedef int **grid_t;\ngrid_t g;\nunion value *v;\nenum status s;\n";
    let extraction = extract("src/types.c", Language::C, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Type};
    assert_eq!(
        symbols,
        [
            ("money", Type),
            ("point", Type),
            ("intptr", Type),
            ("status", Type),
            ("status_t", Type),
            ("node", Class),
            ("node_t", Type),
            ("node_ptr", Type),
            ("a", Type),
            ("b", Type),
            ("cint", Type),
            ("label", Type)
        ]
    );
    let refs: Vec<(&str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| (reference.dst_name.as_str(), reference.line))
        .collect();
    assert_eq!(refs, [("grid_t", 10), ("value", 11), ("status", 12)]);
    Ok(())
}

/// Google Test's tests are tests wherever they are, named by the test inside its suite; a function
/// named `test_` is a test only in a test file, and `testing` lacks the underscore.
#[test]
fn cpp_tests_are_marked_by_macro_or_name() -> TestResult {
    use SymbolKind::{Function, Test};
    let source = "TEST(OrderTest, Pays) {}\nTEST_F(OrderFixture, Ships) {}\nvoid test_total() {}\n\
                  void helper() {}\nvoid testing() {}\n";
    let symbols = |path: &str| -> Result<Vec<(String, SymbolKind)>, Box<dyn Error>> {
        let extraction = extract(path, Language::Cpp, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| (symbol.qual_name.as_str().to_owned(), symbol.kind))
            .collect())
    };
    let named = |kinds: [SymbolKind; 5]| -> Vec<(String, SymbolKind)> {
        [
            "OrderTest.Pays",
            "OrderFixture.Ships",
            "test_total",
            "helper",
            "testing",
        ]
        .into_iter()
        .map(str::to_owned)
        .zip(kinds)
        .collect()
    };
    assert_eq!(
        symbols("src/order_test.cc")?,
        named([Test, Test, Test, Function, Function])
    );
    assert_eq!(
        symbols("src/order.cc")?,
        named([Test, Test, Function, Function, Function])
    );
    Ok(())
}

/// C++ functions and methods by every way of naming them: returning a reference or a pointer to
/// a pointer, operators, destructors, constructors, specializations, and methods defined outside
/// their class, its template or up to three scopes, which take the innermost one's name. A
/// namespace written `a::b` or `n3::n4::n5` is named by its last part, a specialized class by its
/// template, and a class behind an export macro by its own name. A member constant alone in its
/// declaration is read; two in one declaration, a pure virtual method, a friend function, a
/// deleted constructor, a conversion operator and a lambda are not definitions.
#[test]
fn cpp_functions_and_methods_are_named() -> TestResult {
    let source = "const std::string& name(const Order& o) { return o.name; }\n\
                  Order& Order::operator=(const Order& other) { return *this; }\n\
                  bool operator==(const Order& a, const Order& b) { return true; }\n\
                  namespace a::b { int f() { return 0; } }\n\
                  struct Shape { virtual double area() const = 0; static const int MAX = 9; \
                  static constexpr int SIDES = 4, EDGES = 4; };\n\
                  template <typename T> void Stack<T>::pop() {}\nint **grid() { return 0; }\n\
                  class Order final : public shop::Base, Priced { Order(int n) : n_(n) {} ~Order() {} };\n\
                  auto make = [](int x) { return x; };\n\
                  template <> class Hash<Order> { void k() {} };\n\
                  template <> struct std::hash<Order> { int operator()(const Order& o) const { return 0; } };\n\
                  struct W { friend bool operator==(const W&, const W&) { return true; } W(const W&) = delete; };\n\
                  template <> int largest<int>(int a, int b) { return a; }\n\
                  namespace n3::n4::n5 { void deeper() {} }\n\
                  class API Exported { void a() {} };\n\
                  void shop::Order::Inner::h() {}\nvoid a::B::f() {}\n\
                  struct Conv { operator bool() const { return true; } };\n";
    let extraction = extract("src/order.cpp", Language::Cpp, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant, Function, Method, Module};
    assert_eq!(
        symbols,
        [
            ("name", Function),
            ("Order.operator=", Method),
            ("operator==", Function),
            ("b", Module),
            ("b.f", Function),
            ("Shape", Class),
            ("Shape.MAX", Constant),
            ("Stack.pop", Method),
            ("grid", Function),
            ("Order", Class),
            ("Order.Order", Method),
            ("Order.~Order", Method),
            ("Hash", Class),
            ("Hash.k", Method),
            ("hash", Class),
            ("hash.operator()", Method),
            ("W", Class),
            ("largest", Function),
            ("n5", Module),
            ("n5.deeper", Function),
            ("Exported", Class),
            ("Exported.a", Method),
            ("Inner.h", Method),
            ("B.f", Method),
            ("Conv", Class)
        ]
    );
    Ok(())
}

/// C++ calls: plain, through an object or a pointer, qualified once or twice, with template
/// arguments, and creating an object with `new`, plain or qualified once or twice. A cast is
/// not a call.
#[test]
fn cpp_calls_are_read_in_every_form() -> TestResult {
    let source = "void g(Order* p, Order o) {\n    pay(1);\n    o.ship();\n    p->bill();\n    \
                  std::max(1, 2);\n    shop::util::log(1);\n    make<int>(1);\n    \
                  std::make_unique<Order>();\n    new Order();\n    new shop::Receipt();\n    \
                  static_cast<int>(1);\n    std::chrono::duration_cast<std::chrono::seconds>(d);\n    \
                  new a::b::C();\n}\n";
    let extraction = extract("src/g.cpp", Language::Cpp, source)?;
    let calls: Vec<(&str, u32)> = extraction
        .refs
        .iter()
        .filter(|reference| reference.kind == RefKind::Call)
        .map(|reference| (reference.dst_name.as_str(), reference.line))
        .collect();
    assert_eq!(
        calls,
        [
            ("pay", 2),
            ("ship", 3),
            ("bill", 4),
            ("max", 5),
            ("log", 6),
            ("make", 7),
            ("make_unique", 8),
            ("Order", 9),
            ("Receipt", 10),
            ("duration_cast", 12),
            ("C", 13)
        ]
    );
    Ok(())
}

/// C++ base classes and type uses qualified by up to two scopes, plain or with template
/// arguments. The export macro before a class's name is no type.
#[test]
fn cpp_qualified_types_are_read() -> TestResult {
    let source = "class E2 : public a::b::C {};\na::b::C cc;\nstd::chrono::seconds s;\n\
                  std::vector<shop::Order> orders;\nclass API Exported { void a() {} };\n";
    let extraction = extract("src/types.cpp", Language::Cpp, source)?;
    let refs: Vec<(&str, &str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| {
            (
                reference.kind.name(),
                reference.dst_name.as_str(),
                reference.line,
            )
        })
        .collect();
    assert_eq!(
        refs,
        [
            ("inherit", "C", 1),
            ("type_ref", "C", 2),
            ("type_ref", "seconds", 3),
            ("type_ref", "vector", 4),
            ("type_ref", "Order", 4)
        ]
    );
    Ok(())
}

/// C++ reads what C does with the same patterns: `#define` constants and macros but no include
/// guard, typedefs that never nest, unions, constant pointers, a forward declaration as a use, no
/// `const` inside a function, and no call inside an attribute.
#[test]
fn cpp_shares_the_c_readings() -> TestResult {
    let source = "#ifndef ORDER_H\n#define ORDER_H\n#define LIMIT 3\n\
                  #define MAX(a, b) ((a) > (b) ? (a) : (b))\ntypedef unsigned long money;\n\
                  typedef struct node { int v; } node_t, *node_ptr;\n\
                  typedef int cint __attribute__((aligned(8)));\n\
                  union amount { int cents; float units; };\nclass Receipt;\n\
                  char *const LABEL = nullptr;\nvoid pay(Receipt* r) { const int fee = 1; }\n#endif\n";
    let extraction = extract("src/order.hpp", Language::Cpp, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant, Function, Type};
    assert_eq!(
        symbols,
        [
            ("LIMIT", Constant),
            ("MAX", Function),
            ("money", Type),
            ("node", Class),
            ("node_t", Type),
            ("node_ptr", Type),
            ("cint", Type),
            ("amount", Class),
            ("LABEL", Constant),
            ("pay", Function)
        ]
    );
    let refs: Vec<(&str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| (reference.dst_name.as_str(), reference.line))
        .collect();
    assert_eq!(refs, [("Receipt", 9), ("Receipt", 11)]);
    Ok(())
}

/// C# tests are methods with a test attribute of NUnit, xUnit or MSTest, with or without its
/// `Attribute` suffix, arguments or other attributes, wherever the file is. A qualified attribute
/// (`Moq.Test`) could be any library's and is not read; no name makes a test.
#[test]
fn csharp_tests_are_marked_by_attribute() -> TestResult {
    use SymbolKind::{Class, Method, Test};
    let source = "class OrderTests\n{\n    [Test] public void A() { }\n    [Fact] public void B() { }\n    \
                  [Theory] [InlineData(1)] public void C(int n) { }\n    [TestMethod] public void D() { }\n    \
                  [TestCase(1)] [TestCase(2)] public void E(int n) { }\n    \
                  [DataTestMethod] public void F() { }\n    [TestAttribute] public void G() { }\n    \
                  [Moq.Test] public void H() { }\n    [Obsolete] public void TestI() { }\n    \
                  public void Helper() { }\n}\n";
    for path in ["tests/OrderTests.cs", "src/Order.cs"] {
        let extraction = extract(path, Language::CSharp, source)?;
        let kinds: Vec<SymbolKind> = extraction
            .symbols
            .iter()
            .map(|symbol| symbol.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                Class, Test, Test, Test, Test, Test, Test, Test, Method, Method, Method
            ],
            "{path}"
        );
    }
    Ok(())
}

/// A C# file-scoped namespace, `namespace Shop.Orders;`, names the types after it, as a block
/// namespace does by holding them.
#[test]
fn csharp_file_scoped_namespaces_qualify_types() -> TestResult {
    let source = "namespace Shop.Orders;\npublic class Order { public void Pay() { } }\n\
                  public interface IPriced { }\npublic delegate void Paid();\npublic struct Money { }\n\
                  public record Line(int X);\npublic enum Status { Open }\n";
    let extraction = extract("src/Order.cs", Language::CSharp, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Interface, Method, Module, Type};
    assert_eq!(
        symbols,
        [
            ("Shop.Orders", Module),
            ("Shop.Orders.Order", Class),
            ("Shop.Orders.Order.Pay", Method),
            ("Shop.Orders.IPriced", Interface),
            ("Shop.Orders.Paid", Type),
            ("Shop.Orders.Money", Class),
            ("Shop.Orders.Line", Class),
            ("Shop.Orders.Status", Class)
        ]
    );
    Ok(())
}

/// C# constants are `const` fields and `static readonly` ones, the two modifiers in either order
/// and among others; a field only `readonly` or only `static` is not one.
#[test]
fn csharp_constants_cannot_change() -> TestResult {
    let source = "class C\n{\n    public const int A = 1;\n    static readonly int B = 2;\n    \
                  readonly static int D = 3;\n    public static readonly string E = \"e\";\n    \
                  readonly int F = 4;\n    static int G = 5;\n}\n";
    let extraction = extract("src/C.cs", Language::CSharp, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant};
    assert_eq!(
        symbols,
        [
            ("C", Class),
            ("C.A", Constant),
            ("C.B", Constant),
            ("C.D", Constant),
            ("C.E", Constant)
        ]
    );
    Ok(())
}

/// C# type uses in a cast, a qualified declaration, a nullable and an array, after `as`, in a
/// method's result and in nested or qualified type arguments; calls with type arguments, plain or
/// through a type, on an object that may be null, and creating a qualified type. `nameof` is no
/// call.
#[test]
fn csharp_types_and_calls_in_every_form() -> TestResult {
    let source = "class D\n{\n    void M(object obj)\n    {\n        var x = (Item)obj;\n        \
                  System.Text.StringBuilder sb = new System.Text.StringBuilder();\n        \
                  Item? i = null;\n        Item[] arr = null;\n        Run<int>();\n        \
                  Helper.Run<Order>();\n        a?.Pay();\n        var z = obj as Zed;\n        \
                  Dictionary<string, List<Line>> d;\n        nameof(Foo);\n        \
                  List<System.Text.Encoding> e;\n    }\n    Task<Item> Load() => null;\n}\n";
    let extraction = extract("src/D.cs", Language::CSharp, source)?;
    let refs: Vec<(&str, &str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| {
            (
                reference.kind.name(),
                reference.dst_name.as_str(),
                reference.line,
            )
        })
        .collect();
    assert_eq!(
        refs,
        [
            ("type_ref", "Item", 5),
            ("type_ref", "StringBuilder", 6),
            ("call", "StringBuilder", 6),
            ("type_ref", "Item", 7),
            ("type_ref", "Item", 8),
            ("call", "Run", 9),
            ("call", "Run", 10),
            ("type_ref", "Order", 10),
            ("call", "Pay", 11),
            ("type_ref", "Zed", 12),
            ("type_ref", "Dictionary", 13),
            ("type_ref", "List", 13),
            ("type_ref", "Line", 13),
            ("type_ref", "List", 15),
            ("type_ref", "Encoding", 15),
            ("type_ref", "Task", 17),
            ("type_ref", "Item", 17)
        ]
    );
    Ok(())
}

/// C# base types, qualified or given arguments by a record, and structs and records as classes.
#[test]
fn csharp_bases_and_records() -> TestResult {
    let source =
        "class X : System.IDisposable { }\nrecord R1(int A) : Base1(A), IFoo1;\nstruct S { }\n";
    let extraction = extract("src/X.cs", Language::CSharp, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::Class;
    assert_eq!(symbols, [("X", Class), ("R1", Class), ("S", Class)]);
    let refs: Vec<(&str, &str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| {
            (
                reference.kind.name(),
                reference.dst_name.as_str(),
                reference.line,
            )
        })
        .collect();
    assert_eq!(
        refs,
        [
            ("inherit", "IDisposable", 1),
            ("inherit", "Base1", 2),
            ("inherit", "IFoo1", 2)
        ]
    );
    Ok(())
}

/// A name written across several tokens keeps only the tokens: a comment or a line break inside
/// a C# namespace or `using` is not part of it, and nothing inside is lost to it.
#[test]
fn csharp_names_drop_comments_and_line_breaks() -> TestResult {
    let source = "using System /* io */ .IO;\nnamespace A /* hi */ . B { class Y { } }\n\
                  namespace Shop\n    .Orders\n{ class X { void M() { } } }\n";
    let extraction = extract("src/X.cs", Language::CSharp, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Method, Module};
    assert_eq!(
        symbols,
        [
            ("A.B", Module),
            ("A.B.Y", Class),
            ("Shop.Orders", Module),
            ("Shop.Orders.X", Class),
            ("Shop.Orders.X.M", Method)
        ]
    );
    let imports: Vec<&str> = extraction
        .imports
        .iter()
        .map(|import| import.specifier.as_str())
        .collect();
    assert_eq!(imports, ["System.IO"]);
    // A file-scoped namespace's name, which the types after it take as their qualifier.
    let scoped = extract(
        "src/Z.cs",
        Language::CSharp,
        "namespace Pay /* x */ . Ments;\nclass Z { }\n",
    )?;
    let names: Vec<&str> = scoped
        .symbols
        .iter()
        .map(|symbol| symbol.qual_name.as_str())
        .collect();
    assert_eq!(names, ["Pay.Ments", "Pay.Ments.Z"]);
    Ok(())
}

/// A C++ operator's or destructor's name keeps no spacing from the source, except between two
/// words: `operator <` is `operator<`, `~ Wi` is `~Wi`, `operator new` keeps its space.
#[test]
fn cpp_operator_names_ignore_spacing() -> TestResult {
    let source = "struct Wi {\n    bool operator <(const Wi& o) const { return true; }\n    \
                  void* operator new(unsigned long n) { return 0; }\n    ~ Wi() {}\n};\n";
    let extraction = extract("src/wi.cpp", Language::Cpp, source)?;
    let names: Vec<&str> = extraction
        .symbols
        .iter()
        .map(|symbol| symbol.qual_name.as_str())
        .collect();
    assert_eq!(names, ["Wi", "Wi.operator<", "Wi.operator new", "Wi.~Wi"]);
    Ok(())
}

/// PHPUnit's tests: a method with the `Test` attribute, plain or PHPUnit's own qualified one, or
/// with `@test` in the comment just before it, anywhere; one named `test...` in a test file. Another
/// library's `Test` attribute and `@testing` mark nothing.
#[test]
fn php_tests_are_marked_by_attribute_or_name() -> TestResult {
    use SymbolKind::{Class, Method, Test};
    let source = "<?php\nclass OrderTest extends TestCase\n{\n    public function testPays(): void { }\n    \
                  #[Test]\n    public function ships(): void { }\n    \
                  /** @test */\n    public function it_bills(): void { }\n    \
                  #[\\PHPUnit\\Framework\\Attributes\\Test]\n    public function refunds(): void { }\n    \
                  #[Other\\Test]\n    public function later(): void { }\n    \
                  /** @testing */\n    public function helper(): void { }\n}\n";
    let kinds = |path: &str| -> Result<Vec<SymbolKind>, Box<dyn Error>> {
        let extraction = extract(path, Language::Php, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| symbol.kind)
            .collect())
    };
    assert_eq!(
        kinds("src/OrderTest.php")?,
        [Class, Test, Test, Test, Test, Method, Method]
    );
    assert_eq!(
        kinds("src/Order.php")?,
        [Class, Method, Test, Test, Test, Method, Method]
    );
    Ok(())
}

/// Kotlin's tests: a function annotated as a test by JUnit, kotlin.test or TestNG, plainly or by
/// its qualified name, with arguments or without, in any file; none goes by its name. Another
/// annotation marks nothing.
#[test]
fn kotlin_tests_are_marked_by_annotation() -> TestResult {
    use SymbolKind::{Class, Method, Test};
    let source = "class OrderTest {\n    @Test\n    fun pays() {}\n\n    \
                  @Test(expected = IllegalStateException::class)\n    fun refuses() {}\n\n    \
                  @ParameterizedTest\n    @ValueSource(ints = [1, 2])\n    fun `bills twice`(n: Int) {}\n\n    \
                  @org.junit.jupiter.api.RepeatedTest(3)\n    fun ships() {}\n\n    \
                  @TestFactory\n    fun cases() = listOf<DynamicTest>()\n\n    \
                  @TestTemplate\n    fun template() {}\n\n    \
                  @Tested\n    fun helper() {}\n\n    fun testLater() {}\n}\n";
    let symbols = |path: &str| -> Result<Vec<(String, SymbolKind)>, Box<dyn Error>> {
        let extraction = extract(path, Language::Kotlin, source)?;
        Ok(extraction
            .symbols
            .iter()
            .map(|symbol| (symbol.qual_name.as_str().to_owned(), symbol.kind))
            .collect())
    };
    let expected = [
        ("OrderTest".to_owned(), Class),
        ("OrderTest.pays".to_owned(), Test),
        ("OrderTest.refuses".to_owned(), Test),
        ("OrderTest.`bills twice`".to_owned(), Test),
        ("OrderTest.ships".to_owned(), Test),
        ("OrderTest.cases".to_owned(), Test),
        ("OrderTest.template".to_owned(), Test),
        ("OrderTest.helper".to_owned(), Method),
        ("OrderTest.testLater".to_owned(), Method),
    ];
    assert_eq!(symbols("src/test/kotlin/OrderTest.kt")?, expected);
    assert_eq!(symbols("src/main/kotlin/Order.kt")?, expected);
    Ok(())
}

/// Kotlin declarations of every kind: interfaces, `fun interface`s and annotation classes are
/// interfaces; data, enum, sealed, value, nested and inner classes, objects and named companion
/// objects are classes, and an unnamed companion object holds its members for its class. Members
/// are methods, in an enum and its entries too, and in an anonymous object, where no class holds
/// them; local and extension functions are functions; `const val` is a constant wherever it is,
/// a plain `val` is none; a type alias is a type.
#[test]
fn kotlin_declarations_of_every_kind() -> TestResult {
    let source = "fun interface Handler {\n    fun handle()\n}\n\
                  sealed interface Shape\n\
                  annotation class Fast\n\
                  @JvmInline\nvalue class Cents(val amount: Int)\n\
                  enum class Suit {\n    HEARTS {\n        override fun color() = 1\n    },\n    SPADES;\n\n    \
                  open fun color() = 0\n}\n\
                  class Repo {\n    class Row\n    inner class Cursor\n    \
                  companion object Factory {\n        fun make() = Repo()\n    }\n}\n\
                  class Cache {\n    companion object {\n        const val SIZE = 4\n    }\n}\n\
                  object Config {\n    const val NAME = \"x\"\n    val timeout = 5\n}\n\
                  val handler = object : Handler {\n    override fun handle() {}\n}\n\
                  fun Repo.extra(): Int {\n    fun inner() = 1\n    return inner()\n}\n\
                  typealias Rows = List<Repo.Row>\n";
    let extraction = extract("src/kinds.kt", Language::Kotlin, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant, Function, Interface, Method, Type};
    assert_eq!(
        symbols,
        [
            ("Handler", Interface),
            ("Handler.handle", Method),
            ("Shape", Interface),
            ("Fast", Interface),
            ("Cents", Class),
            ("Suit", Class),
            ("Suit.color", Method),
            ("Suit.color", Method),
            ("Repo", Class),
            ("Repo.Row", Class),
            ("Repo.Cursor", Class),
            ("Repo.Factory", Class),
            ("Repo.Factory.make", Method),
            ("Cache", Class),
            ("Cache.SIZE", Constant),
            ("Config", Class),
            ("Config.NAME", Constant),
            ("handle", Method),
            ("extra", Function),
            ("extra.inner", Function),
            ("Rows", Type)
        ]
    );
    Ok(())
}

/// Kotlin references in every form: calls plain, generic, with only a trailing lambda, on an
/// object, on a maybe-null object, on `super` and through a qualified name, and infix between
/// two names; bases plain, generic, qualified, given arguments and delegated, generic or not;
/// uses of a type nullable, generic, qualified, in a function
/// type, a bound, a receiver, after `is` and `as?`, in a `catch` and in an annotation with
/// arguments. A member reference (`Order::price`) and a property (`order.size`) are no calls.
#[test]
fn kotlin_references_in_every_form() -> TestResult {
    let source = "class Box<T : Comparable<T>>(val item: T) : java.io.Serializable, Base<T>(), \
                  Comparable<Box<T>>, Logs by Logger(), Iterable<T> by items {\n    \
                  @Fast(level = 2)\n    fun run(order: Order?, f: (Int) -> Unit): kotlin.collections.List<Line> {\n        \
                  listOf<Int>().map { it }\n        lock { }\n        order?.pay()\n        super.toString()\n        \
                  Outer.Inner.make()\n        val pair = start to end\n        if (order is Paid) { }\n        \
                  val price = order as? Shop.Price\n        try { } catch (e: IllegalStateException) { }\n        \
                  val ref = Order::price\n        val size = order.size\n        return emptyList()\n    }\n\n    \
                  fun Order.total(): Int = 0\n}\n";
    let extraction = extract("src/box.kt", Language::Kotlin, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Method};
    assert_eq!(
        symbols,
        [("Box", Class), ("Box.run", Method), ("Box.total", Method)]
    );
    let refs: Vec<(&str, &str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| {
            (
                reference.kind.name(),
                reference.dst_name.as_str(),
                reference.line,
            )
        })
        .collect();
    assert_eq!(
        refs,
        [
            ("type_ref", "Comparable", 1),
            ("type_ref", "T", 1),
            ("type_ref", "T", 1),
            ("inherit", "Serializable", 1),
            ("inherit", "Base", 1),
            ("type_ref", "T", 1),
            ("inherit", "Comparable", 1),
            ("type_ref", "Box", 1),
            ("type_ref", "T", 1),
            ("inherit", "Logs", 1),
            ("call", "Logger", 1),
            ("inherit", "Iterable", 1),
            ("type_ref", "T", 1),
            ("type_ref", "Fast", 2),
            ("type_ref", "Order", 3),
            ("type_ref", "Int", 3),
            ("type_ref", "Unit", 3),
            ("type_ref", "List", 3),
            ("type_ref", "Line", 3),
            ("call", "listOf", 4),
            ("type_ref", "Int", 4),
            ("call", "map", 4),
            ("call", "lock", 5),
            ("call", "pay", 6),
            ("call", "toString", 7),
            ("call", "make", 8),
            ("call", "to", 9),
            ("type_ref", "Paid", 10),
            ("type_ref", "Price", 11),
            ("type_ref", "IllegalStateException", 12),
            ("call", "emptyList", 15),
            ("type_ref", "Order", 18),
            ("type_ref", "Int", 18)
        ]
    );
    Ok(())
}

/// A class or object that delegates (`: MutableMap<String, Int> by m`) keeps its body where the
/// grammar reads the body as a lambda passed to the delegate, which Kotlin does not allow there:
/// the header ends before it, its functions are methods, and the delegate's name, plain or reached
/// through another, is no call, while a delegate given arguments is one. Each is the last
/// declaration of its file, where the grammar reads it so.
#[test]
fn kotlin_delegating_classes_keep_their_bodies() -> TestResult {
    type Read = (Vec<(String, SymbolKind, ShortHash)>, Vec<String>);
    let read = |source: &str| -> Result<Read, Box<dyn Error>> {
        let extraction = extract("src/store.kt", Language::Kotlin, source)?;
        let symbols = extraction
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.qual_name.as_str().to_owned(),
                    symbol.kind,
                    symbol.sig_hash,
                )
            })
            .collect();
        let calls = extraction
            .refs
            .iter()
            .filter(|reference| reference.kind == RefKind::Call)
            .map(|reference| reference.dst_name.as_str().to_owned())
            .collect();
        Ok((symbols, calls))
    };
    let header = |text: &str| ShortHash::of(text.as_bytes());
    let put = "override fun put(key: String, value: Int): Int?";
    use SymbolKind::{Class, Method};
    let (symbols, calls) = read(
        "class Counts(private val m: MutableMap<String, Int>) : MutableMap<String, Int> by m {\n    \
         override fun put(key: String, value: Int): Int? = m.put(key, value)\n}\n",
    )?;
    let counts =
        "class Counts(private val m: MutableMap<String, Int>) : MutableMap<String, Int> by m";
    assert_eq!(
        symbols,
        [
            ("Counts".to_owned(), Class, header(counts)),
            ("Counts.put".to_owned(), Method, header(put))
        ]
    );
    assert_eq!(calls, ["put"]);
    let (symbols, calls) = read(
        "class Nav(private val m: MutableMap<String, Int>) : MutableMap<String, Int> by Holder.m {\n    \
         override fun put(key: String, value: Int): Int? = m.put(key, value)\n}\n",
    )?;
    let nav =
        "class Nav(private val m: MutableMap<String, Int>) : MutableMap<String, Int> by Holder.m";
    assert_eq!(
        symbols,
        [
            ("Nav".to_owned(), Class, header(nav)),
            ("Nav.put".to_owned(), Method, header(put))
        ]
    );
    assert_eq!(calls, ["put"]);
    let (symbols, calls) = read(
        "class Wrap(private val m: MutableMap<String, Int>) : MutableMap<String, Int> by Wrapped(m) {\n    \
         override fun put(key: String, value: Int): Int? = m.put(key, value)\n}\n",
    )?;
    let wrap = "class Wrap(private val m: MutableMap<String, Int>) : MutableMap<String, Int> by Wrapped(m)";
    assert_eq!(
        symbols,
        [
            ("Wrap".to_owned(), Class, header(wrap)),
            ("Wrap.put".to_owned(), Method, header(put))
        ]
    );
    assert_eq!(calls, ["Wrapped", "put"]);
    let (symbols, calls) = read(
        "object Names : List<String> by names {\n    override fun isEmpty() = false\n\n    \
         @Fast\n    fun later() = 1\n}\n",
    )?;
    assert_eq!(
        symbols,
        [
            (
                "Names".to_owned(),
                Class,
                header("object Names : List<String> by names")
            ),
            (
                "Names.isEmpty".to_owned(),
                Method,
                header("override fun isEmpty()")
            ),
            (
                "Names.later".to_owned(),
                Method,
                header("@Fast fun later()")
            )
        ]
    );
    assert!(calls.is_empty(), "{calls:?}");
    Ok(())
}

/// A Kotlin script (`build.gradle.kts`) is read as statements: its calls, infix ones included,
/// belong to no definition.
#[test]
fn kotlin_scripts_are_read() -> TestResult {
    let source = "plugins {\n    kotlin(\"jvm\") version \"2.0.0\"\n}\n\n\
                  dependencies {\n    implementation(\"org.example:lib:1.0\")\n}\n";
    let extraction = extract("build.gradle.kts", Language::Kotlin, source)?;
    assert!(extraction.symbols.is_empty());
    let refs: Vec<(&str, &str, u32, Option<usize>)> = extraction
        .refs
        .iter()
        .map(|reference| {
            (
                reference.kind.name(),
                reference.dst_name.as_str(),
                reference.line,
                reference.symbol,
            )
        })
        .collect();
    assert_eq!(
        refs,
        [
            ("call", "plugins", 1, None),
            ("call", "kotlin", 2, None),
            ("call", "version", 2, None),
            ("call", "dependencies", 5, None),
            ("call", "implementation", 6, None)
        ]
    );
    Ok(())
}

/// Kotlin imports in every form: a name from a package of one part or of several, or with no
/// package, each renamed or not; every name of a package of one part, two or three; an import
/// ended by `;`, and one with a comment inside its name.
#[test]
fn kotlin_imports_in_every_form() -> TestResult {
    let source = "import x.Y\nimport p.q.r.s.T;\nimport single\nimport single as S\nimport a.b.D as E\n\
                  import a.b.*\nimport m./* note */n.O\nimport k.L as M\nimport p.q.r.*\nimport solo.*\n";
    let extraction = extract("src/imports.kt", Language::Kotlin, source)?;
    let imports: Vec<(&str, String, Option<&str>, u32)> = extraction
        .imports
        .iter()
        .map(|import| {
            let imported = match &import.imported {
                Imported::Name(name) => format!("name:{}", name.as_str()),
                other => format!("{other:?}"),
            };
            let alias = import.alias.as_ref().map(cortex_core::Identifier::as_str);
            (import.specifier.as_str(), imported, alias, import.line)
        })
        .collect();
    assert_eq!(
        imports,
        [
            ("x", "name:Y".to_owned(), None, 1),
            ("p.q.r.s", "name:T".to_owned(), None, 2),
            ("single", "Module".to_owned(), None, 3),
            ("single", "Module".to_owned(), Some("S"), 4),
            ("a.b", "name:D".to_owned(), Some("E"), 5),
            ("a.b", "All".to_owned(), None, 6),
            ("m.n", "name:O".to_owned(), None, 7),
            ("k", "name:L".to_owned(), Some("M"), 8),
            ("p.q.r", "All".to_owned(), None, 9),
            ("solo", "All".to_owned(), None, 10)
        ]
    );
    Ok(())
}

/// A PHP namespace with a block holds what is in it, of every kind; the global block has no name.
/// `use` takes a name from a namespace, or names a class of the global namespace, renamed or
/// not; an include reads a literal path, and a path a function builds names no file. A
/// constant's value is not a definition.
#[test]
fn php_namespaces_and_includes() -> TestResult {
    let source = "<?php\nnamespace App\\Billing {\n    use App\\Models\\Order;\n    class Invoice { }\n    \
                  enum Status { }\n    interface Payable { }\n    trait Audits { }\n    const LIMIT = MAX;\n}\n\
                  namespace {\n    use Item as I;\n    include 'a.php';\n    \
                  require dirname(__FILE__) . '/c.php';\n    function main() { }\n}\n";
    let extraction = extract("src/billing.php", Language::Php, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant, Function, Interface, Module};
    assert_eq!(
        symbols,
        [
            ("App\\Billing", Module),
            ("App\\Billing.Invoice", Class),
            ("App\\Billing.Status", Class),
            ("App\\Billing.Payable", Interface),
            ("App\\Billing.Audits", Interface),
            ("App\\Billing.LIMIT", Constant),
            ("main", Function)
        ]
    );
    let imports: Vec<(&str, String, Option<&str>, u32)> = extraction
        .imports
        .iter()
        .map(|import| {
            let imported = match &import.imported {
                Imported::Name(name) => format!("name:{}", name.as_str()),
                other => format!("{other:?}"),
            };
            let alias = import.alias.as_ref().map(cortex_core::Identifier::as_str);
            (import.specifier.as_str(), imported, alias, import.line)
        })
        .collect();
    assert_eq!(
        imports,
        [
            ("App\\Models", "name:Order".to_owned(), None, 3),
            ("Item", "Module".to_owned(), Some("I"), 11),
            ("a.php", "Module".to_owned(), None, 12)
        ]
    );
    Ok(())
}

/// A PHP include of each kind names its file by a literal path, quoted either way, or by a path
/// relative to the file's own folder, either one perhaps in parentheses. A path built otherwise, or
/// with a variable or an escape inside its quotes, names none.
#[test]
fn php_includes_of_every_kind_and_form() -> TestResult {
    let forms = [
        ("'a.php'", Some("a.php")),
        ("\"a.php\"", Some("a.php")),
        ("('a.php')", Some("a.php")),
        ("(\"a.php\")", Some("a.php")),
        ("__DIR__ . '/a.php'", Some("/a.php")),
        ("__DIR__ . \"/a.php\"", Some("/a.php")),
        ("(__DIR__ . '/a.php')", Some("/a.php")),
        ("(__DIR__ . \"/a.php\")", Some("/a.php")),
        ("ROOT . '/a.php'", None),
        ("dirname(__FILE__) . '/a.php'", None),
        ("\"a$version.php\"", None),
        ("\"a\\n.php\"", None),
        ("$path", None),
    ];
    for kind in ["require", "require_once", "include", "include_once"] {
        for (form, expected) in forms {
            let source = format!("<?php\n{kind} {form};\n");
            let extraction = extract("src/boot.php", Language::Php, &source)?;
            let specifiers: Vec<&str> = extraction
                .imports
                .iter()
                .map(|import| import.specifier.as_str())
                .collect();
            let expected: Vec<&str> = expected.into_iter().collect();
            assert_eq!(specifiers, expected, "{kind} {form}");
        }
    }
    Ok(())
}

/// After a second PHP `namespace` statement, the declarations that follow take the nearer one, of
/// every kind: classes, enums, interfaces, traits, functions and constants, two in one statement.
/// A constant's value (`C1` in `C3 = C1`) and an enum's cases are not definitions.
#[test]
fn php_namespaces_after_namespaces() -> TestResult {
    let source = "<?php\nnamespace A;\nclass X {}\nnamespace B;\nclass Y { public function m() {} }\n\
                  function f() {}\nconst C1 = 1, C2 = 2;\nconst C3 = C1;\nenum Suit { case Hearts; }\n\
                  interface I {}\n\
                  trait T {}\n";
    let extraction = extract("src/both.php", Language::Php, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant, Function, Interface, Method, Module};
    assert_eq!(
        symbols,
        [
            ("A", Module),
            ("A.X", Class),
            ("B", Module),
            ("B.Y", Class),
            ("B.Y.m", Method),
            ("B.f", Function),
            ("B.C1", Constant),
            ("B.C2", Constant),
            ("B.C3", Constant),
            ("B.Suit", Class),
            ("B.I", Interface),
            ("B.T", Interface)
        ]
    );
    Ok(())
}

/// PHP references and imports in every form: qualified bases, interfaces, traits, calls and types,
/// a call on a maybe-null object, `instanceof` with a plain or a qualified class, `define` in
/// another case and with double quotes, its value no constant, and `defined`, which defines
/// nothing; `self`, `static` and `parent` name no type or class. A `use` with a leading backslash,
/// `use function`, an alias, and includes quoted either way, in parentheses or relative to the
/// file's folder; a path built from a constant names no file.
#[test]
fn php_references_and_imports_in_every_form() -> TestResult {
    let source = "<?php\nuse \\Lead\\Thing;\nuse function App\\Support\\format;\nuse App\\Models\\Item as I;\n\
                  require_once 'lib.php';\ninclude_once __DIR__ . \"/dq.php\";\nrequire ROOT . '/x.php';\n\
                  require('paren.php');\ndefined('UP') || DEFINE(\"UP\", 'up');\n\
                  class C extends \\App\\Base implements \\App\\Priced { use \\App\\Logs;\n    \
                  public function m(\\App\\Item|self $i): static {\n        \
                  $a?->pay(); \\App\\log(1); new \\App\\Order(); new static();\n        \
                  if ($i instanceof \\App\\Line || $i instanceof Lot) {}\n    }\n}\n";
    let extraction = extract("src/c.php", Language::Php, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant, Method};
    assert_eq!(symbols, [("UP", Constant), ("C", Class), ("C.m", Method)]);
    let refs: Vec<(&str, &str, u32)> = extraction
        .refs
        .iter()
        .map(|reference| {
            (
                reference.kind.name(),
                reference.dst_name.as_str(),
                reference.line,
            )
        })
        .collect();
    assert_eq!(
        refs,
        [
            ("call", "defined", 9),
            ("call", "DEFINE", 9),
            ("inherit", "Base", 10),
            ("inherit", "Priced", 10),
            ("inherit", "Logs", 10),
            ("type_ref", "Item", 11),
            ("call", "pay", 12),
            ("call", "log", 12),
            ("call", "Order", 12),
            ("type_ref", "Line", 13),
            ("type_ref", "Lot", 13)
        ]
    );
    let imports: Vec<(&str, String, Option<&str>, u32)> = extraction
        .imports
        .iter()
        .map(|import| {
            let imported = match &import.imported {
                Imported::Name(name) => format!("name:{}", name.as_str()),
                other => format!("{other:?}"),
            };
            let alias = import.alias.as_ref().map(cortex_core::Identifier::as_str);
            (import.specifier.as_str(), imported, alias, import.line)
        })
        .collect();
    assert_eq!(
        imports,
        [
            ("Lead", "name:Thing".to_owned(), None, 2),
            ("App\\Support", "name:format".to_owned(), None, 3),
            ("App\\Models", "name:Item".to_owned(), Some("I"), 4),
            ("lib.php", "Module".to_owned(), None, 5),
            ("/dq.php", "Module".to_owned(), None, 6),
            ("paren.php", "Module".to_owned(), None, 8)
        ]
    );
    Ok(())
}

/// A PHP `use` group takes each name from its prefix, a qualified name whole.
#[test]
fn php_grouped_uses_keep_qualified_names() -> TestResult {
    let source = "<?php\nuse Models\\{Order, Sub\\Item, Line as L};\n";
    let extraction = extract("src/uses.php", Language::Php, source)?;
    let imports: Vec<(&str, String, Option<&str>)> = extraction
        .imports
        .iter()
        .map(|import| {
            let imported = match &import.imported {
                Imported::Name(name) => format!("name:{}", name.as_str()),
                other => format!("{other:?}"),
            };
            let alias = import.alias.as_ref().map(cortex_core::Identifier::as_str);
            (import.specifier.as_str(), imported, alias)
        })
        .collect();
    assert_eq!(
        imports,
        [
            ("Models", "name:Order".to_owned(), None),
            ("Models", "name:Sub\\Item".to_owned(), None),
            ("Models", "name:Line".to_owned(), Some("L"))
        ]
    );
    Ok(())
}

/// Java constants are `static final` fields, with the modifiers in either order, and the fields
/// of an interface; other fields are not definitions.
#[test]
fn java_constants_take_either_modifier_order() -> TestResult {
    let source = "class C {\n    static final int A = 1;\n    final static int B = 2;\n    \
                  static int c = 3;\n    final int d = 4;\n}\ninterface I {\n    int E = 5;\n}\n";
    let extraction = extract("shop/C.java", Language::Java, source)?;
    let symbols: Vec<(&str, SymbolKind)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.kind))
        .collect();
    use SymbolKind::{Class, Constant, Interface};
    assert_eq!(
        symbols,
        [
            ("C", Class),
            ("C.A", Constant),
            ("C.B", Constant),
            ("I", Interface),
            ("I.E", Constant)
        ]
    );
    Ok(())
}

/// A definition whose qualified name would pass the 256-byte limit on stored names is left out
/// with everything inside it, and a name used inside it belongs to the nearest definition kept.
#[test]
fn long_qualified_names_are_left_out() -> TestResult {
    let (a, b, c) = ("A".repeat(100), "B".repeat(100), "C".repeat(100));
    let source = format!(
        "class {a}:\n    class {b}:\n        class {c}:\n            def method(self):\n                \
         helper()\n"
    );
    // A is 100 bytes long and A.B 201; A.B.C would be 302, so C and its method are left out.
    let extraction = extract("shop/deep.py", Language::Python, &source)?;
    let symbols: Vec<(&str, Option<usize>)> = extraction
        .symbols
        .iter()
        .map(|symbol| (symbol.qual_name.as_str(), symbol.parent))
        .collect();
    assert_eq!(
        symbols,
        [(a.as_str(), None), (format!("{a}.{b}").as_str(), Some(0))]
    );
    let refs: Vec<(&str, u32, Option<usize>)> = extraction
        .refs
        .iter()
        .map(|reference| {
            (
                reference.dst_name.as_str(),
                reference.line,
                reference.symbol,
            )
        })
        .collect();
    assert_eq!(refs, [("helper", 5, Some(1))]);
    Ok(())
}

/// A definition or an import broken by a syntax error is skipped and the rest of the file is still
/// read, and bytes that are not UTF-8 outside names change nothing.
#[test]
fn broken_input_keeps_the_rest() -> TestResult {
    let extraction = extract(
        "shop/broken.py",
        Language::Python,
        "import a.$b\nimport c\n\ndef (x):\n    pass\n\ndef g():\n    pass\n",
    )?;
    let names: Vec<&str> = extraction
        .symbols
        .iter()
        .map(|symbol| symbol.qual_name.as_str())
        .collect();
    assert_eq!(names, ["g"]);
    let modules: Vec<&str> = extraction
        .imports
        .iter()
        .map(|import| import.specifier.as_str())
        .collect();
    assert_eq!(modules, ["c"]);
    let mut extractor = extractor(GENEROUS)?;
    let latin1 = extractor.extract(
        &RepoPath::new("shop/latin1.py")?,
        Language::Python,
        b"NAME = \"caf\xe9\"\ndef f():\n    return NAME\n",
    )?;
    let names: Vec<&str> = latin1
        .symbols
        .iter()
        .map(|symbol| symbol.qual_name.as_str())
        .collect();
    assert_eq!(names, ["NAME", "f"]);
    Ok(())
}

/// A file that takes longer than allowed is reported, not read; with time it reads in full.
#[test]
fn slow_files_time_out() -> TestResult {
    let source = "def f(x):\n    return g(x) + 1\n".repeat(5000);
    let path = RepoPath::new("shop/big.py")?;
    assert_eq!(
        extractor(Duration::ZERO)?.extract(&path, Language::Python, source.as_bytes()),
        Err(ExtractError::TimedOut { path: path.clone() })
    );
    let extraction = extractor(GENEROUS)?.extract(&path, Language::Python, source.as_bytes())?;
    assert_eq!(extraction.symbols.len(), 5000);
    assert_eq!(extraction.refs.len(), 5000);
    Ok(())
}

/// A language Cortex does not parse is refused by name.
#[test]
fn unparsed_languages_are_refused() -> TestResult {
    let path = RepoPath::new("data.json")?;
    assert_eq!(
        extractor(GENEROUS)?.extract(&path, Language::Json, b"{}"),
        Err(ExtractError::NotParsed {
            language: Language::Json
        })
    );
    Ok(())
}
