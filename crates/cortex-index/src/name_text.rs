//! The text of a name as its tokens spell it. A name can span several tokens (`Shop.Orders`,
//! `operator <`, `a . b`), and the source may put comments, line breaks and spacing between them;
//! those are not part of the name. The tokens are joined with nothing between them, or with one
//! space where two words would otherwise run together (`operator new`).

use tree_sitter::Node;

/// The name `node` holds in `source`: the text of each token under it that is not a comment,
/// joined as the module says. `None` when a token's text is not UTF-8.
pub(crate) fn name_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.child_count() == 0 {
        return node.utf8_text(source).ok().map(str::to_owned);
    }
    let mut tokens = Vec::new();
    let mut cursor = node.walk();
    loop {
        let current = cursor.node();
        if current.child_count() == 0 {
            if !current.is_extra() {
                tokens.push(current.utf8_text(source).ok()?);
            }
        } else if !current.is_extra() && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() || cursor.node() == node {
                return Some(joined(&tokens));
            }
        }
    }
}

/// `tokens` joined with nothing between them, or one space where the token before ends and the
/// one after starts with a word character.
fn joined(tokens: &[&str]) -> String {
    let mut text = String::new();
    for token in tokens {
        let meets_word = text.chars().next_back().is_some_and(is_word_character)
            && token.chars().next().is_some_and(is_word_character);
        if meets_word {
            text.push(' ');
        }
        text.push_str(token);
    }
    text
}

/// Whether `character` can be part of a word in a name: a letter, a digit or an underscore.
fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::test_support::parse_python;

    /// Tokens join directly, and with a space only where two words meet.
    #[test]
    fn tokens_join_as_a_name() {
        assert_eq!(joined(&["Shop", ".", "Orders"]), "Shop.Orders");
        assert_eq!(joined(&["operator", "<"]), "operator<");
        assert_eq!(joined(&["operator", "new"]), "operator new");
        assert_eq!(joined(&["~", "Order"]), "~Order");
        assert_eq!(joined(&["total", "="]), "total=");
        assert_eq!(joined(&[]), "");
    }

    /// A dotted name loses the spacing and the line continuation between its parts; a single
    /// token is read as it is.
    #[test]
    fn names_lose_what_lies_between_their_tokens() -> Result<(), Box<dyn Error>> {
        // In `import a . b \ . c`, the dotted name runs from `a` to `c`, with a line
        // continuation, which the grammar keeps as an extra, before the second dot.
        let source = "import a . b \\\n . c\nx\n";
        let tree = parse_python(source)?;
        let root = tree.root_node();
        let dotted = root
            .child(0)
            .and_then(|import| import.child_by_field_name("name"))
            .ok_or("no dotted name")?;
        assert_eq!(
            name_text(dotted, source.as_bytes()).as_deref(),
            Some("a.b.c")
        );
        let single = root
            .child(1)
            .and_then(|statement| statement.child(0))
            .ok_or("no identifier")?;
        assert_eq!(name_text(single, source.as_bytes()).as_deref(), Some("x"));
        Ok(())
    }
}
