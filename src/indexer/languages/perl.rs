//! Perl language extractor (regex-based, enhanced).
//
//! Extracts packages, subroutines, constants, Moose/Moo OOP patterns,
//! and use/require/parent/base import statements from Perl source code.
//
//! tree-sitter-perl is not available as a compatible crate for tree-sitter 0.25.x,
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

    // No matching brace found (e.g. single-line sub or forward declaration)
    start_line + fallback_offset
}

/// Estimate cyclomatic complexity for a Perl subroutine body (regex heuristic).
fn estimate_complexity_perl(source: &str, start_byte: usize, end_byte: usize) -> u32 {
    let end = end_byte.min(source.len());
    if start_byte >= end {
        return 1;
    }
    let body = &source[start_byte..end];
    let mut complexity: u32 = 1; // base

    let decision_re = Regex::new(r"\b(if|elsif|unless|while|until|for|foreach|when)\b").unwrap();
    complexity += decision_re.find_iter(body).count() as u32;

    // Count logical operators
    complexity += body.matches("&&").count() as u32;
    complexity += body.matches("||").count() as u32;

    complexity
}
/// Extract nodes and edges from Perl source code using regex.
pub fn extract_regex(file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // -----------------------------------------------------------------------
    // 1. Packages: package Name; or package Name VERSION;
    // -----------------------------------------------------------------------
    let package_re = Regex::new(r"(?m)^\s*package\s+([\w:]+)").unwrap();
    for caps in package_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 10);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Module,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({}),
        });
    }

    // -----------------------------------------------------------------------
    // 2. Subroutines: sub name { ... } or sub name : method { ... }
    // -----------------------------------------------------------------------
    let sub_re = Regex::new(r"(?m)^\s*sub\s+(\w+)(?:\s*:\s*(\w+))?").unwrap();
    for caps in sub_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let attr = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 5);
        let end_byte = source
            .lines()
            .take(end_line as usize)
            .map(|l| l.len() + 1)
            .sum::<usize>();
        let complexity = estimate_complexity_perl(source, match_start, end_byte);
        let attributes = if attr == "method" {
            json!({"method_attribute": true, "complexity": complexity})
        } else {
            json!({"complexity": complexity})
        };
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes,
        });
    }

    // -----------------------------------------------------------------------
    // 3. Constants: use constant NAME => value;
    // -----------------------------------------------------------------------
    let const_re = Regex::new(r"(?m)^\s*use\s+constant\s+(\w+)\s*=>").unwrap();
    for caps in const_re.captures_iter(source) {
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
            attributes: json!({}),
        });
    }

    // -----------------------------------------------------------------------
    // 4. Moose/Moo class detection: when a package uses Moose or Moo,
    //    emit a Class node for the package.
    // -----------------------------------------------------------------------
    let moose_re = Regex::new(r"(?m)^\s*use\s+(Moose|Moo|Mouse)\b").unwrap();
    if moose_re.is_match(source)
        && let Some(pkg_caps) = package_re.captures(source)
    {
        let pkg_name = pkg_caps.get(1).unwrap().as_str();
        let match_start = pkg_caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line(source, match_start, line, 50);
        nodes.push(Node {
            fqn: format!("{}::{}::__class__", file, pkg_name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"moose": true, "package": pkg_name}),
        });
    }

    // -----------------------------------------------------------------------
    // 5. Moose/Moo attributes: has 'attr_name' => (...)
    // -----------------------------------------------------------------------
    let has_re = Regex::new(r#"(?m)^\s*has\s+['"]?(\w+)['"]?\s*=>"#).unwrap();
    for caps in has_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line: line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"moose_attr": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 6. OOP: bless with literal class name -> emit Class node
    // -----------------------------------------------------------------------
    let bless_re = Regex::new(r#"(?m)\bbless\s+[^,]+,\s*['"](\w+)['"]"#).unwrap();
    for caps in bless_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line: line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"bless": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 7. Inheritance via @ISA: our @ISA = ('Base');
    // -----------------------------------------------------------------------
    let isa_re = Regex::new(r#"(?m)@ISA\s*=\s*\(([^)]+)\)"#).unwrap();
    let base_re = Regex::new(r#"['"](\w[\w:]*)['"]"#).unwrap();
    for caps in isa_re.captures_iter(source) {
        let bases_str = caps.get(1).unwrap().as_str();
        for base_caps in base_re.captures_iter(bases_str) {
            let base = base_caps.get(1).unwrap().as_str();
            edges.push(Edge {
                id: None,
                source_fqn: file.to_string(),
                target_fqn: base.to_string(),
                kind: EdgeKind::Inherits,
                confidence: 1.0,
                attributes: json!({"via": "@ISA"}),
            });
        }
    }

    // -----------------------------------------------------------------------
    // 8. Use statements with special handling for parent/base/constant/pragmas
    // -----------------------------------------------------------------------
    let use_re = Regex::new(r#"(?m)^\s*use\s+([\w:]+)(?:\s+['"](\w[\w:]*)['"])?"#).unwrap();
    for caps in use_re.captures_iter(source) {
        let target = caps.get(1).unwrap().as_str();
        match target {
            "parent" | "base" => {
                if let Some(base_cap) = caps.get(2) {
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: base_cap.as_str().to_string(),
                        kind: EdgeKind::Inherits,
                        confidence: 1.0,
                        attributes: json!({"via": target}),
                    });
                }
            }
            "strict" | "warnings" | "utf8" | "feature" | "lib" | "constant" | "Moose" | "Moo"
            | "Mouse" => {
                // Skip pragmas and OOP frameworks (handled separately)
            }
            _ => {
                // Skip version strings like v5.10
                if target.starts_with('v')
                    && target[1..].chars().all(|c| c.is_ascii_digit() || c == '.')
                {
                    // version pragma, skip
                } else {
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: target.to_string(),
                        kind: EdgeKind::Imports,
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // 9. Require statements: require Module or require "file"
    // -----------------------------------------------------------------------
    let require_re = Regex::new(r#"(?m)^\s*require\s+(?:['"]([^'"]+)['"]|([\w:]+))"#).unwrap();
    for caps in require_re.captures_iter(source) {
        let target = caps.get(1).or_else(|| caps.get(2)).unwrap().as_str();
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({}),
        });
    }

    // -----------------------------------------------------------------------
    // 10. Moose/Moo extends: extends 'Base' -> Inherits edge
    // -----------------------------------------------------------------------
    let extends_re = Regex::new(r#"(?m)^\s*extends\s+['"](\w[\w:]*)['"]"#).unwrap();
    for caps in extends_re.captures_iter(source) {
        let base = caps.get(1).unwrap().as_str();
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: base.to_string(),
            kind: EdgeKind::Inherits,
            confidence: 1.0,
            attributes: json!({"via": "extends"}),
        });
    }

    // -----------------------------------------------------------------------
    // 11. Moose/Moo with: with 'Role' -> Implements edge
    // -----------------------------------------------------------------------
    let with_re = Regex::new(r#"(?m)^\s*with\s+['"](\w[\w:]*)['"]"#).unwrap();
    for caps in with_re.captures_iter(source) {
        let role = caps.get(1).unwrap().as_str();
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: role.to_string(),
            kind: EdgeKind::Implements,
            confidence: 1.0,
            attributes: json!({"via": "with"}),
        });
    }

    // -----------------------------------------------------------------------
    // 12. Simple function call extraction (identifier followed by parentheses)
    // -----------------------------------------------------------------------
    let call_re = Regex::new(r"(?m)\b([a-zA-Z_]\w*)\s*\(").unwrap();
    let perl_keywords: std::collections::HashSet<&str> = [
        "if", "elsif", "else", "unless", "while", "until", "for", "foreach", "do", "sub", "my",
        "our", "local", "return", "use", "require", "package", "BEGIN", "END", "eval", "die",
        "warn", "print", "say", "open", "close", "chomp", "chop", "push", "pop", "shift",
        "unshift", "splice", "grep", "map", "sort", "join", "split", "defined", "exists", "delete",
        "ref", "bless", "new", "has", "extends", "with",
    ]
    .iter()
    .copied()
    .collect();

    // Find function ranges for enclosing context
    let fn_ranges: Vec<(&str, u32, u32)> = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .map(|n| (n.fqn.as_str(), n.start_line, n.end_line))
        .collect();

    for caps in call_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        if perl_keywords.contains(name) {
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
            attributes: json!({"call_type": "function"}),
        });
    }

    ExtractionResult { nodes, edges }
}
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Baseline tests (preserved from original)
    // ------------------------------------------------------------------

    #[test]
    fn test_perl_extract_subs_packages_imports() {
        let source = r#"
package MyApp::UserService;

use strict;
use warnings;
use DBI;
use MyApp::Config qw(get_config);

sub new {
    my ($class, %args) = @_;
    return bless \%args, $class;
}

sub find_user {
    my ($self, $id) = @_;
    return $id;
}

sub validate {
    my ($self, $data) = @_;
    return defined $data;
}

1;
"#;
        let result = extract_regex("lib/MyApp/UserService.pm", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/MyApp/UserService.pm::MyApp::UserService"
                    && n.kind == NodeKind::Module)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/MyApp/UserService.pm::new" && n.kind == NodeKind::Function)
        );
        assert!(result.nodes.iter().any(
            |n| n.fqn == "lib/MyApp/UserService.pm::find_user" && n.kind == NodeKind::Function
        ));
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/MyApp/UserService.pm::validate"
                    && n.kind == NodeKind::Function)
        );

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "DBI"));
        assert!(imports.iter().any(|e| e.target_fqn == "MyApp::Config"));
        assert!(!imports.iter().any(|e| e.target_fqn == "strict"));
        assert!(!imports.iter().any(|e| e.target_fqn == "warnings"));
    }

    #[test]
    fn test_perl_empty_file() {
        let result = extract_regex("empty.pl", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    // ------------------------------------------------------------------
    // New tests for enhanced constructs
    // ------------------------------------------------------------------

    #[test]
    fn test_perl_constants() {
        let source = r#"
package MyApp;

use constant MAX_SIZE => 100;
use constant PI => 3.14159;
use constant APP_NAME => "MyApp";
"#;
        let result = extract_regex("lib/MyApp.pm", source);

        let max_size = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/MyApp.pm::MAX_SIZE");
        assert!(max_size.is_some(), "MAX_SIZE constant not found");
        assert_eq!(max_size.unwrap().kind, NodeKind::Constant);

        let pi = result.nodes.iter().find(|n| n.fqn == "lib/MyApp.pm::PI");
        assert!(pi.is_some(), "PI constant not found");
        assert_eq!(pi.unwrap().kind, NodeKind::Constant);

        let app_name = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/MyApp.pm::APP_NAME");
        assert!(app_name.is_some(), "APP_NAME constant not found");
        assert_eq!(app_name.unwrap().kind, NodeKind::Constant);
    }

    #[test]
    fn test_perl_moose_class_detection() {
        let source = r#"
package Animal;

use Moose;

has 'name' => (is => 'rw', isa => 'Str');
has 'sound' => (is => 'ro', isa => 'Str');

sub speak {
    my $self = shift;
    print $self->name;
}

1;
"#;
        let result = extract_regex("lib/Animal.pm", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/Animal.pm::Animal" && n.kind == NodeKind::Module)
        );

        let class_node = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Class && n.attributes["moose"] == true);
        assert!(class_node.is_some(), "Moose class node not found");

        let name_attr = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/Animal.pm::name" && n.attributes["moose_attr"] == true);
        assert!(name_attr.is_some(), "Moose has attribute name not found");

        let sound_attr = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/Animal.pm::sound" && n.attributes["moose_attr"] == true);
        assert!(sound_attr.is_some(), "Moose has attribute sound not found");
    }

    #[test]
    fn test_perl_moose_extends_inherits() {
        let source = r#"
package Dog;

use Moose;
extends 'Animal';

sub bark {
    my $self = shift;
    print "Woof!";
}

1;
"#;
        let result = extract_regex("lib/Dog.pm", source);

        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            inherits.iter().any(|e| e.target_fqn == "Animal"),
            "Inherits edge to Animal not found"
        );
    }

    #[test]
    fn test_perl_moose_with_role() {
        let source = r#"
package Cat;

use Moose;
extends 'Animal';
with 'Printable';
with 'Serializable';

1;
"#;
        let result = extract_regex("lib/Cat.pm", source);

        let implements: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Implements)
            .collect();
        assert!(
            implements.iter().any(|e| e.target_fqn == "Printable"),
            "Implements edge to Printable not found"
        );
        assert!(
            implements.iter().any(|e| e.target_fqn == "Serializable"),
            "Implements edge to Serializable not found"
        );

        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            inherits.iter().any(|e| e.target_fqn == "Animal"),
            "Inherits edge to Animal not found"
        );
    }

    #[test]
    fn test_perl_isa_inheritance() {
        let source = r#"
package MyClass;

our @ISA = ('BaseClass', 'Mixin');

sub new {
    my $class = shift;
    return bless {}, $class;
}

1;
"#;
        let result = extract_regex("lib/MyClass.pm", source);

        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            inherits.iter().any(|e| e.target_fqn == "BaseClass"),
            "Inherits edge to BaseClass not found"
        );
        assert!(
            inherits.iter().any(|e| e.target_fqn == "Mixin"),
            "Inherits edge to Mixin not found"
        );
    }

    #[test]
    fn test_perl_use_parent_base() {
        let source = r#"
package Child;

use parent 'Parent';

1;
"#;
        let source2 = r#"
package Child2;

use base 'Parent2';

1;
"#;
        let result1 = extract_regex("lib/Child.pm", source);
        let result2 = extract_regex("lib/Child2.pm", source2);

        let inherits1: Vec<&Edge> = result1
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            inherits1.iter().any(|e| e.target_fqn == "Parent"),
            "use parent should produce Inherits edge"
        );

        let inherits2: Vec<&Edge> = result2
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            inherits2.iter().any(|e| e.target_fqn == "Parent2"),
            "use base should produce Inherits edge"
        );

        let imports1: Vec<&Edge> = result1
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(
            !imports1.iter().any(|e| e.target_fqn == "parent"),
            "parent should not be an Imports edge"
        );
        assert!(
            !imports1.iter().any(|e| e.target_fqn == "Parent"),
            "Parent should not be an Imports edge"
        );
    }

    #[test]
    fn test_perl_bless_class_detection() {
        let source = r#"
package OldStyle;

sub new {
    my ($class, %args) = @_;
    return bless \%args, 'OldStyle';
}

1;
"#;
        let result = extract_regex("lib/OldStyle.pm", source);

        let class_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/OldStyle.pm::OldStyle" && n.kind == NodeKind::Class);
        assert!(class_node.is_some(), "bless-based Class node not found");
        assert_eq!(class_node.unwrap().attributes["bless"], true);
    }

    #[test]
    fn test_perl_method_attribute() {
        let source = r#"
package MyClass;

use Moose;

sub greet : method {
    my $self = shift;
    return "Hello";
}

sub plain_sub {
    return 42;
}

1;
"#;
        let result = extract_regex("lib/MyClass.pm", source);

        let greet = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/MyClass.pm::greet");
        assert!(greet.is_some(), "greet sub not found");
        assert_eq!(greet.unwrap().kind, NodeKind::Function);
        assert_eq!(greet.unwrap().attributes["method_attribute"], true);

        let plain = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/MyClass.pm::plain_sub");
        assert!(plain.is_some(), "plain_sub not found");
        assert_eq!(
            plain.unwrap().attributes["method_attribute"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn test_perl_end_line_brace_depth() {
        let source = r#"
package Test;

sub outer {
    my $x = 1;
    if ($x > 0) {
        print $x;
    }
}

sub simple {
    return 42;
}

1;
"#;
        let result = extract_regex("lib/Test.pm", source);

        let outer = result.nodes.iter().find(|n| n.fqn == "lib/Test.pm::outer");
        assert!(outer.is_some());
        assert!(
            outer.unwrap().end_line > outer.unwrap().start_line,
            "end_line should be greater than start_line for multi-line sub"
        );

        let simple = result.nodes.iter().find(|n| n.fqn == "lib/Test.pm::simple");
        assert!(simple.is_some());
        assert!(simple.unwrap().end_line >= simple.unwrap().start_line);
    }

    #[test]
    fn test_perl_require_statements() {
        let source = r#"
package MyApp;

require Exporter;
require "some/file.pl";
"#;
        let result = extract_regex("lib/MyApp.pm", source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(
            imports.iter().any(|e| e.target_fqn == "Exporter"),
            "require Exporter not found"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "some/file.pl"),
            "require file not found"
        );
    }

    #[test]
    fn test_perl_mixed_oop_constructs() {
        let source = r#"
package Zoo::Animal;

use Moose;
extends 'LivingThing';
with 'Nameable';

has 'species' => (is => 'ro', isa => 'Str', required => 1);
has 'age' => (is => 'rw', isa => 'Int', default => 0);

use constant MAX_AGE => 200;
use constant DEFAULT_SOUND => "...";

use Scalar::Util qw(blessed);
use List::Util qw(max min);

sub speak : method {
    my $self = shift;
    print $self->sound;
}

sub describe {
    my $self = shift;
    return $self->species;
}

1;
"#;
        let result = extract_regex("lib/Zoo/Animal.pm", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/Zoo/Animal.pm::Zoo::Animal" && n.kind == NodeKind::Module)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.attributes["moose"] == true)
        );
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "lib/Zoo/Animal.pm::species" && n.attributes["moose_attr"] == true
            )
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/Zoo/Animal.pm::age" && n.attributes["moose_attr"] == true)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/Zoo/Animal.pm::MAX_AGE" && n.kind == NodeKind::Constant)
        );
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "lib/Zoo/Animal.pm::DEFAULT_SOUND" && n.kind == NodeKind::Constant
            )
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/Zoo/Animal.pm::speak" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/Zoo/Animal.pm::describe" && n.kind == NodeKind::Function)
        );

        let speak = result
            .nodes
            .iter()
            .find(|n| n.fqn == "lib/Zoo/Animal.pm::speak");
        assert_eq!(speak.unwrap().attributes["method_attribute"], true);

        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(inherits.iter().any(|e| e.target_fqn == "LivingThing"));

        let implements: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Implements)
            .collect();
        assert!(implements.iter().any(|e| e.target_fqn == "Nameable"));

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "Scalar::Util"));
        assert!(imports.iter().any(|e| e.target_fqn == "List::Util"));
    }
}
