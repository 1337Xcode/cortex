//! Workspace lint inheritance.
//!
//! The lints that keep `unwrap`, missing docs and stray prints out of product code are declared once
//! in the workspace manifest and reach a crate only if its own manifest opts in with
//! `[lints] workspace = true`. Cargo reports no lint settings in `cargo metadata`, so this check
//! reads each member's manifest text: a crate that forgets the table would otherwise build and pass
//! clippy with none of the workspace lints applied.

/// Whether a manifest opts into the workspace lints, in any of the three ways Cargo accepts:
/// a `[lints]` table holding `workspace = true`, a top-level `lints.workspace = true`, or an inline
/// `lints = { workspace = true }`. Comments and spaces are ignored.
pub(crate) fn inherits_workspace_lints(manifest: &str) -> bool {
    let mut table = String::new();
    for raw in manifest.lines() {
        let line: String = raw
            .split('#')
            .next()
            .unwrap_or_default()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if line.starts_with('[') {
            table = line;
            continue;
        }
        let opts_in = match table.as_str() {
            "" => line == "lints.workspace=true" || line == "lints={workspace=true}",
            "[lints]" => line == "workspace=true",
            _ => false,
        };
        if opts_in {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three forms Cargo accepts all count.
    #[test]
    fn accepted_forms_inherit() {
        assert!(inherits_workspace_lints(
            "[package]\nname = \"a\"\n\n[lints]\nworkspace = true\n"
        ));
        assert!(inherits_workspace_lints(
            "lints.workspace = true\n[package]\nname = \"a\"\n"
        ));
        assert!(inherits_workspace_lints(
            "lints = { workspace = true }\n[package]\nname = \"a\"\n"
        ));
        assert!(inherits_workspace_lints(
            "[lints]\n  workspace=true  # inherit\n"
        ));
    }

    /// A manifest without the table, or with the key in the wrong place, does not count.
    #[test]
    fn missing_or_misplaced_opt_in_does_not_inherit() {
        assert!(!inherits_workspace_lints("[package]\nname = \"a\"\n"));
        assert!(!inherits_workspace_lints(
            "[lints]\n[lints.rust]\nunsafe_code = \"forbid\"\n"
        ));
        assert!(!inherits_workspace_lints(
            "[package]\nname = \"a\"\nworkspace = true\n"
        ));
        assert!(!inherits_workspace_lints("[lints]\nworkspace = false\n"));
        assert!(!inherits_workspace_lints("# [lints]\n# workspace = true\n"));
    }
}
