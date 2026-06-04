//! Kotlin language extractor (regex-based).
//!
//! Extracts functions, classes, data classes, interfaces, enums, sealed classes,
//! companion objects, extension functions, constants, and imports from Kotlin
//! source code using regex patterns.
//!
//! tree-sitter-kotlin is incompatible with tree-sitter 0.25.x (requires 0.20.x),
//! so this extractor remains regex-based with enhanced pattern coverage.

use regex::Regex;
use serde_json::json;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Estimate the end line of a block starting at `start_byte` in `source` by
/// counting brace depth.  Returns the 1-based line number of the closing `}`.
/// Falls back to `start_line + fallback_offset` when no matching brace is found.
fn estimate_end_line(
    source: &str,
    start_byte: usize,
    start_line: u32,
    fallback_offset: u32,
) -> u32 {
    let slice = &source[start_byte..];
    let mut depth: i32 = 0;
    let mut found_open = false;
    let mut line = start_line;

    for ch in slice.chars() {
        match ch {
            '\n' => line += 1,
            '{' => {
                depth += 1;
                found_open = true;
            }
            '}' => {
                depth -= 1;
                if found_open && depth == 0 {
                    return line;
                }
            }
            _ => {}
        }
    }

    // No matching brace found (e.g. single-expression body or expression body `= ...`)
    start_line + fallback_offset
}

/// Estimate cyclomatic complexity for a Kotlin function body (regex heuristic).
/// Counts decision keywords: if, else if, when, while, for, catch, &&, ||.
fn estimate_complexity(source: &str, start_byte: usize, end_byte: usize) -> u32 {
    let end = end_byte.min(source.len());
    if start_byte >= end {
        return 1;
    }
    let body = &source[start_byte..end];
    let mut complexity: u32 = 1; // base

    // Count decision keywords (word-boundary aware)
    let decision_re = regex::Regex::new(r"\b(if|else\s+if|when|while|for|catch)\b").unwrap();
    complexity += decision_re.find_iter(body).count() as u32;

    // Count logical operators
    complexity += body.matches("&&").count() as u32;
    complexity += body.matches("||").count() as u32;

    complexity
}

/// Extract nodes and edges from Kotlin source code using regex.
pub fn extract_regex(file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // -----------------------------------------------------------------------
    // 1. Enum classes: enum class Name { ... }
    //    Must be matched BEFORE the generic class pattern to avoid double-emit.
    // -----------------------------------------------------------------------
    let enum_re =
        Regex::new(r"(?m)^\s*(?:(?:public|private|internal|protected)\s+)*enum\s+class\s+(\w+)")
            .unwrap();
    for caps in enum_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 5);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Enum,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({}),
        });
    }

    // -----------------------------------------------------------------------
    // 2. Sealed classes: sealed class Name / sealed interface Name
    // -----------------------------------------------------------------------
    let sealed_re = Regex::new(
        r"(?m)^\s*(?:(?:public|private|internal|protected)\s+)*sealed\s+(?:class|interface)\s+(\w+)"
    ).unwrap();
    for caps in sealed_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 10);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sealed": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 3. Data classes: data class Name(...)
    // -----------------------------------------------------------------------
    let data_class_re = Regex::new(
        r"(?m)^\s*(?:(?:public|private|internal|protected|open|abstract)\s+)*data\s+class\s+(\w+)",
    )
    .unwrap();
    for caps in data_class_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 3);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"data_class": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 4. Regular classes (skip enum/sealed/data already handled above)
    // -----------------------------------------------------------------------
    let class_re = Regex::new(
        r"(?m)^\s*(?:(?:public|private|internal|protected|open|abstract)\s+)*class\s+(\w+)",
    )
    .unwrap();
    for caps in class_re.captures_iter(source) {
        let full_match = caps.get(0).unwrap().as_str();
        // Skip variants already handled
        if full_match.contains("data")
            || full_match.contains("enum")
            || full_match.contains("sealed")
        {
            continue;
        }
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 10);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({}),
        });
    }

    // -----------------------------------------------------------------------
    // 5. Interfaces (including sealed interfaces already captured above;
    //    emit plain Interface for non-sealed ones)
    // -----------------------------------------------------------------------
    let iface_re = Regex::new(
        r"(?m)^\s*(?:(?:public|private|internal|protected|functional)\s+)*interface\s+(\w+)",
    )
    .unwrap();
    for caps in iface_re.captures_iter(source) {
        // Skip sealed interfaces (already emitted as Class with sealed=true)
        let prefix_start = caps.get(0).unwrap().start();
        let prefix = &source[prefix_start..caps.get(0).unwrap().end()];
        if prefix.contains("sealed") {
            continue;
        }
        let name = caps.get(1).unwrap().as_str();
        let line = source[..prefix_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, prefix_start, line, 5);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Interface,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({}),
        });
    }

    // -----------------------------------------------------------------------
    // 6. Companion objects: companion object [Name] { ... }
    // -----------------------------------------------------------------------
    let companion_re = Regex::new(r"(?m)^\s*companion\s+object(?:\s+(\w+))?").unwrap();
    for caps in companion_re.captures_iter(source) {
        let name = caps.get(1).map(|m| m.as_str()).unwrap_or("Companion");
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 5);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"companion": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 7. Extension functions: fun ReceiverType.name(...)
    //    Pattern: fun [modifiers] [<generics>] Type[?].name(
    //    Captured groups: (1) receiver_type, (2) function_name
    // -----------------------------------------------------------------------
    // Extension function receiver may include generics: List<Int>, Map<K,V>, etc.
    // Strategy: match the full `fun ... ReceiverType.funcName(` line and extract
    // the last dot-separated segment before `(` as the function name, and
    // everything before that dot as the receiver type.
    // Pattern captures: (1) receiver_type (may include <...>), (2) function_name
    let ext_func_re = Regex::new(
        r"(?m)^\s*(?:(?:public|private|internal|protected|override|suspend|inline|open|abstract)\s+)*fun\s+(?:<[^>]+>\s*)?([\w][\w.<>, ?]*?)\.([\w]+)\s*\("
    ).unwrap();
    for caps in ext_func_re.captures_iter(source) {
        let receiver_type = caps.get(1).unwrap().as_str();
        let name = caps.get(2).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 5);
        let end_byte = source
            .lines()
            .take(end_line as usize)
            .map(|l| l.len() + 1)
            .sum::<usize>();
        let complexity = estimate_complexity(source, match_start, end_byte);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"extension": true, "receiver_type": receiver_type, "complexity": complexity}),
        });
    }

    // -----------------------------------------------------------------------
    // 8. Regular functions (skip extension functions already matched above)
    //    Pattern: fun [modifiers] [<generics>] name(
    // -----------------------------------------------------------------------
    let func_re = Regex::new(
        r"(?m)^\s*(?:(?:public|private|internal|protected|override|suspend|inline|open|abstract)\s+)*fun\s+(?:<[^>]+>\s*)?(\w+)\s*\("
    ).unwrap();
    for caps in func_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();

        // Check if this is actually an extension function by looking at the
        // character immediately before the captured name (should not be a dot)
        let before_name = &source[match_start..caps.get(1).unwrap().start()];
        if before_name.ends_with('.') {
            continue; // extension function, already handled
        }

        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 5);
        let end_byte = source
            .lines()
            .take(end_line as usize)
            .map(|l| l.len() + 1)
            .sum::<usize>();
        let complexity = estimate_complexity(source, match_start, end_byte);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"complexity": complexity}),
        });
    }

    // -----------------------------------------------------------------------
    // 9. Constants
    //    a) const val NAME = ...  (any name)
    //    b) val NAME = ...        (top-level, uppercase name signals a constant)
    // -----------------------------------------------------------------------
    let const_val_re =
        Regex::new(r"(?m)^\s*(?:(?:public|private|internal|protected)\s+)*const\s+val\s+(\w+)")
            .unwrap();
    for caps in const_val_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Constant,
            file: file.to_string(),
            start_line: line,
            end_line: line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"const": true}),
        });
    }

    // Top-level val with SCREAMING_SNAKE_CASE name (heuristic for constants)
    let uppercase_val_re = Regex::new(
        r"(?m)^(?:(?:public|private|internal|protected)\s+)*val\s+([A-Z][A-Z0-9_]+)\s*[=:]",
    )
    .unwrap();
    for caps in uppercase_val_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Constant,
            file: file.to_string(),
            start_line: line,
            end_line: line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"const": false}),
        });
    }

    // -----------------------------------------------------------------------
    // 10. Imports: import package.name or import package.*
    // -----------------------------------------------------------------------
    let import_re = Regex::new(r"(?m)^\s*import\s+([\w]+(?:\.[\w]+)*(?:\.\*)?)").unwrap();
    for caps in import_re.captures_iter(source) {
        let target = caps.get(1).unwrap().as_str();
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            edge_source: crate::store::confidence::EdgeSource::AstDirect,
            attributes: json!({}),
        });
    }

    // -----------------------------------------------------------------------
    // 11. Simple function call extraction (identifier followed by parentheses)
    // -----------------------------------------------------------------------
    let call_re = Regex::new(r"(?m)\b([a-zA-Z_]\w*)\s*\(").unwrap();
    // Keywords to exclude from call detection
    let kotlin_keywords: std::collections::HashSet<&str> = [
        "if",
        "else",
        "when",
        "while",
        "for",
        "do",
        "return",
        "throw",
        "try",
        "catch",
        "finally",
        "class",
        "interface",
        "object",
        "fun",
        "val",
        "var",
        "import",
        "package",
        "enum",
        "sealed",
        "data",
        "abstract",
        "open",
        "override",
        "private",
        "protected",
        "internal",
        "public",
        "companion",
        "suspend",
        "inline",
        "crossinline",
        "noinline",
        "reified",
        "typealias",
        "annotation",
        "constructor",
    ]
    .iter()
    .copied()
    .collect();

    // Collect declared function names for context
    let _declared_fns: std::collections::HashSet<String> = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .filter_map(|n| n.fqn.rsplit("::").next().map(|s| s.to_string()))
        .collect();

    // Determine the enclosing function for each call site
    let mut fn_ranges: Vec<(&str, u32, u32)> = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .map(|n| (n.fqn.as_str(), n.start_line, n.end_line))
        .collect();
    fn_ranges.sort_by_key(|r| r.1);

    for caps in call_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        if kotlin_keywords.contains(name) {
            continue;
        }
        // Skip constructor-like calls (PascalCase) that match class names
        if name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
        {
            continue;
        }

        let match_start = caps.get(0).unwrap().start();
        let call_line = source[..match_start].matches('\n').count() as u32 + 1;

        // Find enclosing function
        let source_fqn = fn_ranges
            .iter()
            .rev()
            .find(|(_, start, end)| call_line >= *start && call_line <= *end)
            .map(|(fqn, _, _)| fqn.to_string())
            .unwrap_or_else(|| file.to_string());

        edges.push(Edge {
            id: None,
            source_fqn,
            target_fqn: name.to_string(),
            kind: EdgeKind::Calls,
            confidence: 0.0,
            edge_source: crate::store::confidence::EdgeSource::AstDirect,
            attributes: json!({"call_type": "function"}),
        });
    }

    ExtractionResult { nodes, edges }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Existing baseline tests (preserved)
    // ------------------------------------------------------------------

    #[test]
    fn test_kotlin_extract_functions_classes_imports() {
        let source = r#"
import kotlinx.coroutines.flow.Flow
import com.example.models.*

data class User(val name: String, val age: Int)

interface Repository {
    fun findAll(): List<User>
}

open class UserService(private val repo: Repository) {
    suspend fun getUsers(): Flow<User> {
        return repo.findAll().asFlow()
    }

    private fun validate(user: User): Boolean {
        return user.name.isNotEmpty()
    }
}

fun main() {
    println("Hello Kotlin")
}
"#;
        let result = extract_regex("src/UserService.kt", source);

        // Check imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(
            imports
                .iter()
                .any(|e| e.target_fqn == "kotlinx.coroutines.flow.Flow")
        );
        assert!(
            imports
                .iter()
                .any(|e| e.target_fqn == "com.example.models.*")
        );

        // Check data class
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/UserService.kt::User" && n.kind == NodeKind::Class)
        );

        // Check interface
        assert!(result.nodes.iter().any(|n| n.fqn == "src/UserService.kt::Repository" && n.kind == NodeKind::Interface));

        // Check class
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/UserService.kt::UserService" && n.kind == NodeKind::Class)
        );

        // Check functions
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/UserService.kt::getUsers" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/UserService.kt::validate" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/UserService.kt::main" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_kotlin_empty_file() {
        let result = extract_regex("empty.kt", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    // ------------------------------------------------------------------
    // New tests for enhanced constructs
    // ------------------------------------------------------------------

    #[test]
    fn test_kotlin_enum_class() {
        let source = r#"
enum class Color {
    RED, GREEN, BLUE
}

enum class Direction(val degrees: Int) {
    NORTH(0), EAST(90), SOUTH(180), WEST(270)
}
"#;
        let result = extract_regex("src/Color.kt", source);

        let color = result.nodes.iter().find(|n| n.fqn == "src/Color.kt::Color");
        assert!(color.is_some(), "Color enum not found");
        assert_eq!(color.unwrap().kind, NodeKind::Enum);

        let direction = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Color.kt::Direction");
        assert!(direction.is_some(), "Direction enum not found");
        assert_eq!(direction.unwrap().kind, NodeKind::Enum);
    }

    #[test]
    fn test_kotlin_sealed_class() {
        let source = r#"
sealed class Result<out T> {
    data class Success<T>(val data: T) : Result<T>()
    data class Error(val message: String) : Result<Nothing>()
    object Loading : Result<Nothing>()
}

sealed interface Event {
    data class Click(val x: Int, val y: Int) : Event
    object Dismiss : Event
}
"#;
        let result = extract_regex("src/Result.kt", source);

        let result_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Result.kt::Result");
        assert!(result_node.is_some(), "Result sealed class not found");
        assert_eq!(result_node.unwrap().kind, NodeKind::Class);
        assert_eq!(result_node.unwrap().attributes["sealed"], true);

        let event_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Result.kt::Event");
        assert!(event_node.is_some(), "Event sealed interface not found");
        assert_eq!(event_node.unwrap().kind, NodeKind::Class);
        assert_eq!(event_node.unwrap().attributes["sealed"], true);
    }

    #[test]
    fn test_kotlin_data_class() {
        let source = r#"
data class User(val name: String, val age: Int)

data class Point(val x: Double, val y: Double) {
    fun distanceTo(other: Point): Double = 0.0
}
"#;
        let result = extract_regex("src/Models.kt", source);

        let user = result.nodes.iter().find(|n| n.fqn == "src/Models.kt::User");
        assert!(user.is_some(), "User data class not found");
        assert_eq!(user.unwrap().kind, NodeKind::Class);
        assert_eq!(user.unwrap().attributes["data_class"], true);

        let point = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Models.kt::Point");
        assert!(point.is_some(), "Point data class not found");
        assert_eq!(point.unwrap().attributes["data_class"], true);
    }

    #[test]
    fn test_kotlin_companion_object() {
        let source = r#"
class MyClass {
    companion object {
        const val TAG = "MyClass"
        fun create(): MyClass = MyClass()
    }
}

class Factory {
    companion object Builder {
        fun build(): Factory = Factory()
    }
}
"#;
        let result = extract_regex("src/MyClass.kt", source);

        // Anonymous companion object gets name "Companion"
        let companion = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/MyClass.kt::Companion");
        assert!(companion.is_some(), "Anonymous companion object not found");
        assert_eq!(companion.unwrap().kind, NodeKind::Class);
        assert_eq!(companion.unwrap().attributes["companion"], true);

        // Named companion object
        let builder = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/MyClass.kt::Builder");
        assert!(
            builder.is_some(),
            "Named companion object Builder not found"
        );
        assert_eq!(builder.unwrap().attributes["companion"], true);
    }

    #[test]
    fn test_kotlin_extension_functions() {
        let source = r#"
fun String.addExclamation(): String = this + "!"

fun List<Int>.sum(): Int {
    return this.fold(0) { acc, i -> acc + i }
}

suspend fun Flow<Int>.collectToList(): List<Int> {
    val result = mutableListOf<Int>()
    collect { result.add(it) }
    return result
}
"#;
        let result = extract_regex("src/Extensions.kt", source);

        let add_excl = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Extensions.kt::addExclamation");
        assert!(add_excl.is_some(), "addExclamation extension not found");
        assert_eq!(add_excl.unwrap().kind, NodeKind::Function);
        assert_eq!(add_excl.unwrap().attributes["extension"], true);
        assert_eq!(add_excl.unwrap().attributes["receiver_type"], "String");

        let sum = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Extensions.kt::sum");
        assert!(sum.is_some(), "sum extension not found");
        assert_eq!(sum.unwrap().attributes["extension"], true);

        let collect = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Extensions.kt::collectToList");
        assert!(collect.is_some(), "collectToList extension not found");
        assert_eq!(collect.unwrap().attributes["extension"], true);
    }

    #[test]
    fn test_kotlin_constants() {
        let source = r#"
const val MAX_SIZE = 100
const val PI = 3.14159
val VERSION = "1.0.0"
val TIMEOUT_MS = 5000L
val API_BASE_URL = "https://api.example.com"
"#;
        let result = extract_regex("src/Constants.kt", source);

        // const val should always be Constant
        let max_size = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Constants.kt::MAX_SIZE");
        assert!(max_size.is_some(), "MAX_SIZE constant not found");
        assert_eq!(max_size.unwrap().kind, NodeKind::Constant);
        assert_eq!(max_size.unwrap().attributes["const"], true);

        let pi = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Constants.kt::PI");
        assert!(pi.is_some(), "PI constant not found");
        assert_eq!(pi.unwrap().kind, NodeKind::Constant);

        // Uppercase val should be treated as constant
        let timeout = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/Constants.kt::TIMEOUT_MS");
        assert!(timeout.is_some(), "TIMEOUT_MS constant not found");
        assert_eq!(timeout.unwrap().kind, NodeKind::Constant);
    }

    #[test]
    fn test_kotlin_end_line_brace_depth() {
        let source = r#"
fun outer() {
    val x = 1
    if (x > 0) {
        println(x)
    }
}

fun simple() = 42
"#;
        let result = extract_regex("src/Test.kt", source);

        let outer = result.nodes.iter().find(|n| n.fqn == "src/Test.kt::outer");
        assert!(outer.is_some());
        // outer starts at line 2, ends at the closing brace on line 7
        assert!(outer.unwrap().end_line >= outer.unwrap().start_line);
        assert!(
            outer.unwrap().end_line > outer.unwrap().start_line,
            "end_line should be greater than start_line for multi-line function"
        );
    }

    #[test]
    fn test_kotlin_no_false_positives_for_enum_in_class() {
        // A regular class that happens to have "enum" in a comment should not
        // be classified as an enum.
        let source = r#"
// This is not an enum class
class MyClass {
    fun doSomething() {}
}
"#;
        let result = extract_regex("src/MyClass.kt", source);

        // Should have a Class node, not an Enum node
        let my_class = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/MyClass.kt::MyClass");
        assert!(my_class.is_some());
        assert_eq!(my_class.unwrap().kind, NodeKind::Class);

        // No Enum nodes should be present
        assert!(!result.nodes.iter().any(|n| n.kind == NodeKind::Enum));
    }

    #[test]
    fn test_kotlin_mixed_constructs() {
        let source = r#"
package com.example

import kotlinx.coroutines.flow.Flow

const val VERSION = "2.0"
val MAX_RETRIES = 3

enum class Status { ACTIVE, INACTIVE, PENDING }

sealed class ApiResponse<out T> {
    data class Success<T>(val data: T) : ApiResponse<T>()
    data class Failure(val error: String) : ApiResponse<Nothing>()
}

interface UserRepository {
    suspend fun getUser(id: String): ApiResponse<User>
}

data class User(val id: String, val name: String)

class UserRepositoryImpl : UserRepository {
    companion object {
        private const val BASE_URL = "https://api.example.com"
    }

    override suspend fun getUser(id: String): ApiResponse<User> {
        return ApiResponse.Success(User(id, "Test"))
    }
}

fun String.toUserId(): String = "user_$this"

fun main() {
    println(VERSION)
}
"#;
        let result = extract_regex("src/App.kt", source);

        // Enum
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/App.kt::Status" && n.kind == NodeKind::Enum)
        );

        // Sealed class
        let api_resp = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/App.kt::ApiResponse");
        assert!(api_resp.is_some());
        assert_eq!(api_resp.unwrap().attributes["sealed"], true);

        // Data classes
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/App.kt::User" && n.attributes["data_class"] == true)
        );

        // Interface
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/App.kt::UserRepository" && n.kind == NodeKind::Interface)
        );

        // Regular class
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/App.kt::UserRepositoryImpl" && n.kind == NodeKind::Class)
        );

        // Companion object
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/App.kt::Companion" && n.attributes["companion"] == true)
        );

        // Extension function
        let to_user_id = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/App.kt::toUserId");
        assert!(to_user_id.is_some());
        assert_eq!(to_user_id.unwrap().attributes["extension"], true);
        assert_eq!(to_user_id.unwrap().attributes["receiver_type"], "String");

        // Constants
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/App.kt::VERSION" && n.kind == NodeKind::Constant)
        );

        // Regular function
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/App.kt::main" && n.kind == NodeKind::Function)
        );

        // Imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(
            imports
                .iter()
                .any(|e| e.target_fqn == "kotlinx.coroutines.flow.Flow")
        );
    }
}
