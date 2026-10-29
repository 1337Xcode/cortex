//! Git pattern text adjusted for the glob engine.
//!
//! Git's patterns have no alternation, so `{a,b}.h` names one odd file, while the glob engine
//! would read it as `a.h` or `b.h`. Braces are escaped before a pattern is compiled; a brace git
//! already sees as escaped (`\{`) stays as it is.

/// `pattern` with every brace that is not already escaped preceded by a backslash.
#[must_use]
pub fn literal_braces(pattern: &str) -> String {
    let mut escaped = String::with_capacity(pattern.len());
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                escaped.push('\\');
                if let Some(next) = characters.next() {
                    escaped.push(next);
                }
            }
            '{' | '}' => {
                escaped.push('\\');
                escaped.push(character);
            }
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plain braces are escaped; escaped ones, and everything else, are left alone.
    #[test]
    fn braces_become_literal() {
        assert_eq!(literal_braces("{a,b}.h"), "\\{a,b\\}.h");
        assert_eq!(literal_braces("\\{x}"), "\\{x\\}");
        assert_eq!(literal_braces("src/**/*.rs"), "src/**/*.rs");
        assert_eq!(literal_braces("a\\\\{"), "a\\\\\\{");
        assert_eq!(literal_braces("trailing\\"), "trailing\\");
    }
}
