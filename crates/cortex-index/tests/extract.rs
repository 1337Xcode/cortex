//! Reading files: definitions, references and imports compared with expectations written by hand
//! from fixtures, then the hashes, test conventions, the limit on qualified names, broken input
//! and the time limit.

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cortex_core::{Language, RepoPath, ShortHash, SymbolKind};
use cortex_index::{ExtractError, Extraction, Extractor, Grammars, Imported};

/// What the tests return: any failure, with its cause.
type TestResult = Result<(), Box<dyn Error>>;

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// An extractor over every grammar, allowing each file `timeout`.
fn extractor(timeout: Duration) -> Result<Extractor, Box<dyn Error>> {
    Ok(Extractor::new(Arc::new(Grammars::load()?), timeout))
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
