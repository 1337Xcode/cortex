//! Line numbers as the index records them: from 1, as editors show them, and small enough to store.

/// The 1-based number of the line at 0-based `row`, when it fits a stored line number.
pub(crate) fn line_number(row: usize) -> Option<u32> {
    u32::try_from(row.checked_add(1)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows count from 0 and lines from 1; a row past the largest stored line has no number.
    #[test]
    fn rows_become_lines() {
        assert_eq!(line_number(0), Some(1));
        assert_eq!(line_number(41), Some(42));
        assert_eq!(line_number(4_294_967_294), Some(u32::MAX));
        assert_eq!(line_number(4_294_967_295), None);
        assert_eq!(line_number(usize::MAX), None);
    }
}
