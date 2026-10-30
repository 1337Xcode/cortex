//! JSON with comments, as tsconfig.json and jsconfig.json are written: a leading byte order mark,
//! `//` and `/* */` comments and trailing commas are taken out, outside strings, before the text
//! is read as JSON.

/// `text` without a leading byte order mark, its comments and its trailing commas; each comment
/// leaves a space, so the tokens on either side stay apart.
pub(super) fn strip(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    without_trailing_commas(&without_comments(text))
}

/// `text` without `//` and `/* */` comments outside strings.
fn without_comments(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    let mut in_string = false;
    while let Some(character) = characters.next() {
        if in_string {
            kept.push(character);
            if character == '\\' {
                kept.extend(characters.next());
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match (character, characters.peek()) {
            ('"', _) => {
                in_string = true;
                kept.push(character);
            }
            ('/', Some('/')) => {
                while characters.next_if(|next| *next != '\n').is_some() {}
                kept.push(' ');
            }
            ('/', Some('*')) => {
                characters.next();
                let mut previous = ' ';
                for next in characters.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
                kept.push(' ');
            }
            _ => kept.push(character),
        }
    }
    kept
}

/// `text` without commas that only whitespace separates from a closing `}` or `]`, outside
/// strings.
fn without_trailing_commas(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut kept = String::with_capacity(text.len());
    let mut in_string = false;
    let mut index = 0;
    while let Some(&character) = characters.get(index) {
        index += 1;
        if in_string {
            kept.push(character);
            if character == '\\' {
                kept.extend(characters.get(index));
                index += 1;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
        }
        if character == ',' {
            let closes = characters
                .get(index..)
                .unwrap_or_default()
                .iter()
                .find(|next| !next.is_whitespace())
                .is_some_and(|next| matches!(next, '}' | ']'));
            if closes {
                continue;
            }
        }
        kept.push(character);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A byte order mark, comments and trailing commas go; the same marks inside strings stay,
    /// after an escaped quote too; what is left reads as JSON.
    #[test]
    fn comments_and_trailing_commas_go() -> Result<(), serde_json::Error> {
        let text = r#"{
  // the compiler's options
  "compilerOptions": {
    "baseUrl": "./src", /* where imports/paths * start */
    "paths": { "@/*": ["*",], "url": ["http://x/*"], },
    "note": "a // not a comment, /* nor this */ and \" quote,]",
  },
}"#;
        let stripped = strip(text);
        let value: serde_json::Value = serde_json::from_str(&stripped)?;
        let options = &value["compilerOptions"];
        assert_eq!(options["baseUrl"], "./src");
        assert_eq!(options["paths"]["@/*"][0], "*");
        assert_eq!(options["paths"]["url"][0], "http://x/*");
        assert_eq!(
            options["note"],
            "a // not a comment, /* nor this */ and \" quote,]"
        );
        let escaped = r#"["a \" b // c"]"#;
        assert_eq!(strip(escaped), escaped);
        assert_eq!(strip("\u{feff}{}"), "{}");
        assert_eq!(strip("1/* */2"), "1 2");
        Ok(())
    }
}
