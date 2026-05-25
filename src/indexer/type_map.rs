//! Local type map for type-aware method call resolution.
//!
//! Maps variable names to their inferred types within a function scope.
//! Built from: type annotations, constructor calls, function return types.
//! This map is NOT persisted to the database; it is scoped to the current indexing run.

use std::collections::HashMap;

/// Maps variable names to their inferred types within a function scope.
///
/// The type map is built during extraction (Pass 1) and consumed during
/// resolution (Pass 2). It enables the resolver to determine receiver types
/// for method calls like `obj.method()`.
///
/// # Sources of type information
/// - Explicit annotations: `let x: Foo = ...` → x is Foo
/// - Constructor calls: `x = Foo()` or `x = new Foo()` → x is Foo
/// - Function return types: `fn bar() -> Foo` → result of bar() is Foo
#[derive(Debug, Clone)]
pub struct LocalTypeMap {
    /// Key: (function_fqn, variable_name), Value: type_name
    bindings: HashMap<(String, String), String>,
}

impl LocalTypeMap {
    /// Creates a new empty LocalTypeMap.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Inserts a type binding for a variable within a function scope.
    ///
    /// # Arguments
    /// * `function_fqn` - The fully qualified name of the containing function
    /// * `variable_name` - The variable name being typed
    /// * `type_name` - The inferred or annotated type name
    pub fn insert(&mut self, function_fqn: String, variable_name: String, type_name: String) {
        self.bindings
            .insert((function_fqn, variable_name), type_name);
    }

    /// Looks up the type of a variable within a function scope.
    ///
    /// # Arguments
    /// * `function_fqn` - The fully qualified name of the containing function
    /// * `variable_name` - The variable name to look up
    ///
    /// # Returns
    /// The type name if a binding exists, or `None` if the variable's type is unknown.
    pub fn get_type(&self, function_fqn: &str, variable_name: &str) -> Option<&str> {
        self.bindings
            .get(&(function_fqn.to_string(), variable_name.to_string()))
            .map(|s| s.as_str())
    }

    /// Returns `true` if the type map contains no bindings.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

impl Default for LocalTypeMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a LocalTypeMap from a source file by scanning for type annotations,
/// constructor calls, and variable assignments.
///
/// This is a language-agnostic heuristic scanner that works across multiple
/// languages by recognizing common patterns:
/// - `let x: Type = ...` (Rust, TypeScript)
/// - `x = Type()` or `x = new Type()` (Python, JS, Java, C#)
/// - `Type x = ...` (Java, C#, C++)
/// - `var x Type = ...` (Go)
pub fn build_type_map_from_source(source: &str, function_fqn: &str) -> LocalTypeMap {
    let mut map = LocalTypeMap::new();

    for line in source.lines() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("--") {
            continue;
        }

        // Pattern: `let varname: TypeName` (Rust, TypeScript)
        if let Some(binding) = extract_let_type_annotation(trimmed) {
            map.insert(function_fqn.to_string(), binding.0, binding.1);
            continue;
        }

        // Pattern: `varname = new TypeName(` or `varname = TypeName(` (Python, JS, Java, C#)
        if let Some(binding) = extract_constructor_assignment(trimmed) {
            map.insert(function_fqn.to_string(), binding.0, binding.1);
            continue;
        }

        // Pattern: `TypeName varname =` or `TypeName varname;` (Java, C#, C++)
        if let Some(binding) = extract_typed_declaration(trimmed) {
            map.insert(function_fqn.to_string(), binding.0, binding.1);
            continue;
        }

        // Pattern: `var varname TypeName` (Go)
        if let Some(binding) = extract_go_var_declaration(trimmed) {
            map.insert(function_fqn.to_string(), binding.0, binding.1);
        }
    }

    map
}

/// Extract type from `let varname: TypeName` or `const varname: TypeName`.
fn extract_let_type_annotation(line: &str) -> Option<(String, String)> {
    let rest = line
        .strip_prefix("let ")
        .or_else(|| line.strip_prefix("const "))
        .or_else(|| line.strip_prefix("var "))?;

    // Find the colon for type annotation
    let colon_pos = rest.find(':')?;
    let var_name = rest[..colon_pos]
        .trim()
        .trim_end_matches("mut ")
        .trim()
        .to_string();
    let after_colon = rest[colon_pos + 1..].trim();

    // Extract the type name (up to `=`, `;`, `<`, or whitespace)
    let type_name: String = after_colon
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    if var_name.is_empty()
        || type_name.is_empty()
        || !type_name.chars().next().is_some_and(|c| c.is_uppercase())
    {
        return None;
    }

    Some((var_name, type_name))
}

/// Extract type from `varname = new TypeName(` or `varname = TypeName(`.
fn extract_constructor_assignment(line: &str) -> Option<(String, String)> {
    let eq_pos = line.find('=')?;
    // Avoid `==`
    if line.get(eq_pos + 1..eq_pos + 2) == Some("=") {
        return None;
    }

    let var_name = line[..eq_pos].trim().to_string();
    let rhs = line[eq_pos + 1..].trim();

    // Strip `new ` prefix if present
    let rhs = rhs.strip_prefix("new ").unwrap_or(rhs).trim_start();

    // Extract type name (uppercase identifier before `(`)
    let paren_pos = rhs.find('(')?;
    let type_name: String = rhs[..paren_pos]
        .trim()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    if var_name.is_empty() || type_name.is_empty() {
        return None;
    }

    // Type names start with uppercase (heuristic to avoid false positives)
    if !type_name.chars().next().is_some_and(|c| c.is_uppercase()) {
        return None;
    }

    // var_name should be a simple identifier
    if !var_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    Some((var_name, type_name))
}

/// Extract type from `TypeName varname =` or `TypeName varname;` (Java/C#/C++ style).
fn extract_typed_declaration(line: &str) -> Option<(String, String)> {
    // Must start with an uppercase letter (type name)
    let first_char = line.chars().next()?;
    if !first_char.is_uppercase() {
        return None;
    }

    // Extract the type name (first word)
    let type_name: String = line
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '<' || *c == '>')
        .collect();

    if type_name.is_empty() {
        return None;
    }

    // Skip generics if present
    let rest = line[type_name.len()..].trim_start();

    // Extract the variable name (next word)
    let var_name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    if var_name.is_empty() {
        return None;
    }

    // Must be followed by `=` or `;` or end of line
    let after_var = rest[var_name.len()..].trim_start();
    if !after_var.starts_with('=') && !after_var.starts_with(';') && !after_var.is_empty() {
        return None;
    }

    Some((var_name, type_name))
}

/// Extract type from `var varname TypeName` (Go style).
fn extract_go_var_declaration(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("var ")?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let var_name = parts[0].to_string();
    let type_name = parts[1].trim_end_matches('=').trim().to_string();

    if var_name.is_empty() || type_name.is_empty() {
        return None;
    }

    // Type name should start with uppercase
    if !type_name.chars().next().is_some_and(|c| c.is_uppercase()) {
        return None;
    }

    Some((var_name, type_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_type_map_is_empty() {
        let map = LocalTypeMap::new();
        assert!(map.is_empty());
    }

    #[test]
    fn insert_and_get_type() {
        let mut map = LocalTypeMap::new();
        map.insert(
            "src/main.rs::process".to_string(),
            "client".to_string(),
            "HttpClient".to_string(),
        );

        let result = map.get_type("src/main.rs::process", "client");
        assert_eq!(result, Some("HttpClient"));
    }

    #[test]
    fn get_type_returns_none_for_unknown_variable() {
        let map = LocalTypeMap::new();
        let result = map.get_type("src/main.rs::process", "unknown");
        assert_eq!(result, None);
    }

    #[test]
    fn get_type_returns_none_for_wrong_function_scope() {
        let mut map = LocalTypeMap::new();
        map.insert(
            "src/main.rs::foo".to_string(),
            "x".to_string(),
            "Foo".to_string(),
        );

        // Same variable name but different function scope
        let result = map.get_type("src/main.rs::bar", "x");
        assert_eq!(result, None);
    }

    #[test]
    fn insert_overwrites_existing_binding() {
        let mut map = LocalTypeMap::new();
        map.insert(
            "src/main.rs::process".to_string(),
            "x".to_string(),
            "OldType".to_string(),
        );
        map.insert(
            "src/main.rs::process".to_string(),
            "x".to_string(),
            "NewType".to_string(),
        );

        let result = map.get_type("src/main.rs::process", "x");
        assert_eq!(result, Some("NewType"));
    }

    #[test]
    fn is_empty_returns_false_after_insert() {
        let mut map = LocalTypeMap::new();
        map.insert(
            "src/lib.rs::init".to_string(),
            "db".to_string(),
            "Database".to_string(),
        );
        assert!(!map.is_empty());
    }

    #[test]
    fn multiple_variables_in_same_function() {
        let mut map = LocalTypeMap::new();
        let fqn = "src/app.rs::handle_request".to_string();

        map.insert(fqn.clone(), "req".to_string(), "Request".to_string());
        map.insert(fqn.clone(), "res".to_string(), "Response".to_string());
        map.insert(fqn.clone(), "db".to_string(), "Database".to_string());

        assert_eq!(
            map.get_type("src/app.rs::handle_request", "req"),
            Some("Request")
        );
        assert_eq!(
            map.get_type("src/app.rs::handle_request", "res"),
            Some("Response")
        );
        assert_eq!(
            map.get_type("src/app.rs::handle_request", "db"),
            Some("Database")
        );
    }

    #[test]
    fn same_variable_name_different_functions() {
        let mut map = LocalTypeMap::new();

        map.insert(
            "src/a.rs::foo".to_string(),
            "x".to_string(),
            "TypeA".to_string(),
        );
        map.insert(
            "src/b.rs::bar".to_string(),
            "x".to_string(),
            "TypeB".to_string(),
        );

        assert_eq!(map.get_type("src/a.rs::foo", "x"), Some("TypeA"));
        assert_eq!(map.get_type("src/b.rs::bar", "x"), Some("TypeB"));
    }

    #[test]
    fn default_creates_empty_map() {
        let map = LocalTypeMap::default();
        assert!(map.is_empty());
    }
}
