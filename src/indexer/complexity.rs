//! Cyclomatic complexity scoring for AST nodes.
//!
//! Counts decision points in a tree-sitter AST subtree to compute
//! cyclomatic complexity. Base complexity is 1 for each function,
//! with +1 for each decision point:
//! - if, else if, elif
//! - match/switch arms
//! - for, while, loop
//! - && (and), || (or)
//! - catch/except
//! - ternary/conditional expressions

use tree_sitter::Node;

/// Compute cyclomatic complexity for a function body subtree.
///
/// Walks all descendant nodes and counts decision points based on
/// the node kinds relevant to the given language.
///
/// Base complexity starts at 1.
pub fn compute_complexity(node: Node, language: &str) -> u32 {
    let mut complexity: u32 = 1; // Base complexity
    count_decision_points(node, language, &mut complexity);
    complexity
}

/// Recursively count decision points in the AST subtree.
fn count_decision_points(node: Node, language: &str, complexity: &mut u32) {
    let kind = node.kind();

    match language {
        "rust" => count_rust_decision_point(kind, complexity),
        "python" => count_python_decision_point(kind, complexity),
        "typescript" | "javascript" | "tsx" => count_typescript_decision_point(kind, complexity),
        "go" => count_go_decision_point(kind, complexity),
        "java" => count_java_decision_point(kind, complexity),
        "c" | "cpp" | "csharp" => count_c_family_decision_point(kind, complexity),
        "ruby" => count_ruby_decision_point(kind, complexity),
        _ => count_generic_decision_point(kind, complexity),
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_decision_points(child, language, complexity);
    }
}

/// Count decision points for Rust code.
fn count_rust_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if_expression" | "else_clause" => *complexity += 1,
        "match_arm" => *complexity += 1,
        "for_expression" | "while_expression" | "loop_expression" => *complexity += 1,
        "binary_expression" => {} // handled via operator check below
        _ => {}
    }
    // Note: && and || are handled as binary_expression children
    // We check for them in the generic handler
}

/// Count decision points for Python code.
fn count_python_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if_statement" | "elif_clause" => *complexity += 1,
        "for_statement" | "while_statement" => *complexity += 1,
        "except_clause" => *complexity += 1,
        "conditional_expression" => *complexity += 1, // ternary: x if cond else y
        "boolean_operator" => *complexity += 1,       // and, or
        "match_statement" => {}                       // match itself doesn't add, arms do
        "case_clause" => *complexity += 1,
        _ => {}
    }
}

/// Count decision points for TypeScript/JavaScript code.
fn count_typescript_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if_statement" => *complexity += 1,
        "else_clause" => {
            // else if counts, plain else doesn't add extra
            // We only count the if_statement inside else
        }
        "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => {
            *complexity += 1
        }
        "switch_case" => *complexity += 1, // each case arm
        "catch_clause" => *complexity += 1,
        "ternary_expression" => *complexity += 1,
        "binary_expression" => {} // && and || handled below
        _ => {}
    }
}

/// Count decision points for Go code.
fn count_go_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if_statement" => *complexity += 1,
        "for_statement" => *complexity += 1,
        "expression_case" => *complexity += 1, // switch case
        "type_case" => *complexity += 1,
        _ => {}
    }
}

/// Count decision points for Java code.
fn count_java_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if_statement" => *complexity += 1,
        "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" => {
            *complexity += 1
        }
        "switch_label" => *complexity += 1,
        "catch_clause" => *complexity += 1,
        "ternary_expression" => *complexity += 1,
        _ => {}
    }
}

/// Count decision points for C/C++/C# code.
fn count_c_family_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if_statement" => *complexity += 1,
        "for_statement" | "while_statement" | "do_statement" => *complexity += 1,
        "case_statement" => *complexity += 1,
        "catch_clause" => *complexity += 1,
        "conditional_expression" => *complexity += 1, // ternary
        _ => {}
    }
}

/// Count decision points for Ruby code.
fn count_ruby_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if" | "elsif" | "unless" => *complexity += 1,
        "for" | "while" | "until" => *complexity += 1,
        "when" => *complexity += 1, // case/when
        "rescue" => *complexity += 1,
        "conditional" => *complexity += 1, // ternary
        _ => {}
    }
}

/// Generic fallback for unsupported languages.
fn count_generic_decision_point(kind: &str, complexity: &mut u32) {
    match kind {
        "if_statement" | "if_expression" | "elif_clause" | "else_clause" => *complexity += 1,
        "for_statement" | "for_expression" | "while_statement" | "while_expression" => {
            *complexity += 1
        }
        "match_arm" | "switch_case" | "case_clause" => *complexity += 1,
        "catch_clause" | "except_clause" => *complexity += 1,
        "ternary_expression" | "conditional_expression" => *complexity += 1,
        _ => {}
    }
}

/// Check if a binary expression node contains && or || operators.
/// This is called separately because we need to inspect the operator text.
pub fn count_logical_operators(node: Node, source: &[u8], language: &str) -> u32 {
    let mut count = 0;
    count_logical_ops_recursive(node, source, language, &mut count);
    count
}

fn count_logical_ops_recursive(node: Node, source: &[u8], language: &str, count: &mut u32) {
    let kind = node.kind();

    match language {
        "rust" | "typescript" | "javascript" | "tsx" | "go" | "java" | "c" | "cpp" | "csharp" => {
            if kind == "binary_expression" {
                // Check the operator child
                if let Some(op_node) = node.child_by_field_name("operator") {
                    let op = op_node.utf8_text(source).unwrap_or("");
                    if op == "&&" || op == "||" {
                        *count += 1;
                    }
                } else {
                    // Some grammars embed the operator as a direct child text
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        let text = child.utf8_text(source).unwrap_or("");
                        if text == "&&" || text == "||" {
                            *count += 1;
                        }
                    }
                }
            }
        }
        // Python uses "boolean_operator" which is already counted in count_python_decision_point
        "python" => {}
        _ => {}
    }

    // Recurse
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_logical_ops_recursive(child, source, language, count);
    }
}

/// Compute full cyclomatic complexity including logical operators.
///
/// This is the main entry point for computing complexity of a function node.
/// It combines structural decision points with logical operator counts.
pub fn compute_full_complexity(node: Node, source: &[u8], language: &str) -> u32 {
    let structural = compute_complexity(node, language);
    let logical_ops = count_logical_operators(node, source, language);
    structural + logical_ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust_source(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    fn parse_python_source(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    fn parse_typescript_source(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_linear_function_complexity_1() {
        let source = r#"
fn linear() -> i32 {
    let x = 1;
    let y = 2;
    x + y
}
"#;
        let tree = parse_rust_source(source);
        let root = tree.root_node();
        // Find the function_item
        let func = root.child(0).unwrap();
        assert_eq!(func.kind(), "function_item");

        let complexity = compute_full_complexity(func, source.as_bytes(), "rust");
        assert_eq!(complexity, 1, "linear function should have complexity 1");
    }

    #[test]
    fn test_nested_if_higher_than_linear() {
        let linear_source = r#"
fn linear() -> i32 {
    let x = 1;
    let y = 2;
    x + y
}
"#;
        let nested_source = r#"
fn nested(x: i32) -> i32 {
    if x > 0 {
        if x > 10 {
            if x > 100 {
                3
            } else {
                2
            }
        } else {
            1
        }
    } else {
        0
    }
}
"#;
        let linear_tree = parse_rust_source(linear_source);
        let nested_tree = parse_rust_source(nested_source);

        let linear_func = linear_tree.root_node().child(0).unwrap();
        let nested_func = nested_tree.root_node().child(0).unwrap();

        let linear_complexity =
            compute_full_complexity(linear_func, linear_source.as_bytes(), "rust");
        let nested_complexity =
            compute_full_complexity(nested_func, nested_source.as_bytes(), "rust");

        assert!(
            nested_complexity > linear_complexity,
            "nested-if ({}) should score higher than linear ({})",
            nested_complexity,
            linear_complexity
        );
    }

    #[test]
    fn test_python_complexity() {
        let source = r#"
def complex_func(x, y):
    if x > 0:
        for i in range(y):
            if i > 5:
                return i
    elif x < 0:
        while y > 0:
            y -= 1
    return 0
"#;
        let tree = parse_python_source(source);
        let root = tree.root_node();
        let func = root.child(0).unwrap();
        assert_eq!(func.kind(), "function_definition");

        let complexity = compute_full_complexity(func, source.as_bytes(), "python");
        // Base 1 + if + for + if + elif + while = 6
        assert!(
            complexity >= 5,
            "complex python function should have complexity >= 5, got {}",
            complexity
        );
    }

    #[test]
    fn test_typescript_complexity() {
        let source = r#"
function complex(x: number): number {
    if (x > 0) {
        for (let i = 0; i < x; i++) {
            if (i > 5) {
                return i;
            }
        }
    }
    return 0;
}
"#;
        let tree = parse_typescript_source(source);
        let root = tree.root_node();
        let func = root.child(0).unwrap();
        assert_eq!(func.kind(), "function_declaration");

        let complexity = compute_full_complexity(func, source.as_bytes(), "typescript");
        // Base 1 + if + for + if = 4
        assert!(
            complexity >= 4,
            "complex typescript function should have complexity >= 4, got {}",
            complexity
        );
    }

    #[test]
    fn test_logical_operators_add_complexity() {
        let source = r#"
fn with_logic(a: bool, b: bool, c: bool) -> bool {
    if a && b || c {
        true
    } else {
        false
    }
}
"#;
        let tree = parse_rust_source(source);
        let root = tree.root_node();
        let func = root.child(0).unwrap();

        let complexity = compute_full_complexity(func, source.as_bytes(), "rust");
        // Base 1 + if + else + && + || = 5
        assert!(
            complexity >= 4,
            "function with logical operators should have complexity >= 4, got {}",
            complexity
        );
    }
}
