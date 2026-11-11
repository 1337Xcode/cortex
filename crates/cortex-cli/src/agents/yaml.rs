//! Text for YAML lines Cortex tells the user to add (it writes no YAML itself): a value in single
//! quotes, where YAML reads every character as it is but a quote, which is doubled.

/// `text` as a YAML scalar in single quotes.
pub(super) fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows path keeps its backslashes, and a quote is doubled.
    #[test]
    fn a_value_is_quoted_by_hand() {
        assert_eq!(
            quoted(r"C:\Users\Ann O'Neil\cortex.exe"),
            r"'C:\Users\Ann O''Neil\cortex.exe'"
        );
        assert_eq!(quoted("/opt/cortex"), "'/opt/cortex'");
    }
}
