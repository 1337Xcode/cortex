//! The text of a name as its tokens spell it. A name can span several tokens (`Shop.Orders`,
//! `operator <`, `a . b`), and the source may put comments, line breaks and spacing between them;
//! those are not part of the name. The tokens are joined with nothing between them, or with one
//! space where two words would otherwise run together (`operator new`). A name some grammars
//! write as a run of sibling nodes with no node around them, as Kotlin writes the package of an
//! import, is read the same way from its first node to its last.
//!
//! A name that is prose, such as a heading's text, is read as written instead, without the spaces
//! that indent it. A grammar can leave such text whole and make tokens only of the markup in
//! it, as Markdown's does with `*`, `` ` `` and `#`, so its tokens would keep the markup and lose
//! the words. Only a query says which names are prose (`@name.prose`): a code name read as
//! written would keep what lies between its tokens.

use tree_sitter::Node;

/// The name `node` holds in `source`: the text of each token under it that is not a comment,
/// joined as the module says. `None` when a token's text is not UTF-8.
pub(crate) fn name_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.child_count() == 0 {
        return node.utf8_text(source).ok().map(str::to_owned);
    }
    let mut tokens = Vec::new();
    push_tokens(node, source, &mut tokens)?;
    Some(joined(&tokens))
}

/// The name `node` holds in `source` when its query marks it as prose: its text as written,
/// without the spaces that indent it, as Markdown keeps those of an underlined heading. `None`
/// when the text is not UTF-8.
pub(crate) fn prose_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    Some(text.trim_start_matches(' ').to_owned())
}

/// The name the siblings from `first` to `last` hold together in `source`, read as
/// [`name_text`] reads one node, with the tokens between them, such as the dots of a dotted
/// name, and without a comment among them. `None` when `last` is not `first` or a sibling
/// after it, or a token's text is not UTF-8.
pub(crate) fn run_text(first: Node<'_>, last: Node<'_>, source: &[u8]) -> Option<String> {
    let mut tokens = Vec::new();
    let mut current = first;
    loop {
        push_tokens(current, source, &mut tokens)?;
        if current == last {
            return Some(joined(&tokens));
        }
        current = current.next_sibling()?;
    }
}

/// Adds to `tokens` the text of each token under `node`, `node` itself when it is one, that is
/// not a comment. `None` when a token's text is not UTF-8.
fn push_tokens<'source>(
    node: Node<'_>,
    source: &'source [u8],
    tokens: &mut Vec<&'source str>,
) -> Option<()> {
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
                return Some(());
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

    use tree_sitter::{Query, QueryCursor, StreamingIterator};

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

    /// Prose reads as written, its markup and the spacing inside it included and the spaces that
    /// indent it left out, where its tokens alone would keep the markup and lose the words.
    #[test]
    fn prose_reads_as_written() -> Result<(), Box<dyn Error>> {
        let source = "## Use the `shop` command\n\n# * *\n\n  Indented  text\n   ---\n";
        let language: tree_sitter::Language = tree_sitter_md::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language)?;
        let tree = parser.parse(source, None).ok_or("no tree")?;
        let headings = "[(atx_heading heading_content: (inline) @text)
                         (setext_heading heading_content: (paragraph (inline) @text))]";
        let query = Query::new(&language, headings)?;
        let mut cursor = QueryCursor::new();
        let mut texts = Vec::new();
        let mut captures = cursor.captures(&query, tree.root_node(), source.as_bytes());
        while let Some((found, index)) = captures.next() {
            let node = found.captures().get(*index).ok_or("no capture")?.node;
            let read = |text: Option<String>| text.unwrap_or_default();
            texts.push((
                read(prose_text(node, source.as_bytes())),
                read(name_text(node, source.as_bytes())),
            ));
        }
        let pair = |prose: &str, tokens: &str| (prose.to_owned(), tokens.to_owned());
        assert_eq!(
            texts,
            [
                pair("Use the `shop` command", "``"),
                pair("* *", "**"),
                pair("Indented  text", "  Indented  text")
            ]
        );
        Ok(())
    }

    /// A run of siblings reads as one name, the dots between them included and a line
    /// continuation among them left out; a run whose last node does not follow its first is
    /// none.
    #[test]
    fn runs_of_siblings_read_as_one_name() -> Result<(), Box<dyn Error>> {
        // The dotted name in `import a . b \ . c` holds `a`, `.`, `b`, the line continuation,
        // `.` and `c`, side by side.
        let source = "import a . b \\\n . c\n";
        let tree = parse_python(source)?;
        let dotted = tree
            .root_node()
            .child(0)
            .and_then(|import| import.child_by_field_name("name"))
            .ok_or("no dotted name")?;
        let mut cursor = dotted.walk();
        let parts: Vec<Node<'_>> = dotted
            .named_children(&mut cursor)
            .filter(|part| part.kind() == "identifier")
            .collect();
        let [a, b, c] = parts[..] else {
            return Err("not three parts".into());
        };
        let text = |first, last| run_text(first, last, source.as_bytes());
        assert_eq!(text(a, c).as_deref(), Some("a.b.c"));
        assert_eq!(text(a, b).as_deref(), Some("a.b"));
        assert_eq!(text(b, b).as_deref(), Some("b"));
        assert_eq!(text(c, a), None);
        Ok(())
    }
}
