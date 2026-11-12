//! What tells one failure from another without keeping it: the short hash of the failing tests a
//! failed run's output named. Two runs that name the same tests, in any order, share it; the names
//! themselves stay in the daemon's reading of the output and are never stored.

use cortex_core::ShortHash;

use super::Failures;

/// The short hash of the failing tests `failures` names, sorted and each once, one name a line:
/// only when the list is known to be whole, as when the counts the runner printed say as many
/// tests failed as there are names. The output is read from its end, and a long run's first
/// failures can fall before it, so a list without counts, or with a failed count of another
/// number, gives none, as does a list naming no test.
#[must_use]
pub fn failing_signature(failures: &Failures) -> Option<ShortHash> {
    let mut names: Vec<&str> = failures.failed_tests.iter().map(String::as_str).collect();
    names.sort_unstable();
    names.dedup();
    let failed = usize::try_from(failures.counts?.failed).ok()?;
    if names.is_empty() || failed != names.len() {
        return None;
    }
    Some(ShortHash::of(names.join("\n").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::CheckCounts;

    /// Failures naming `names`, with counts saying `failed` tests failed, or no counts for none.
    fn naming(names: &[&str], failed: Option<u32>) -> Failures {
        Failures {
            failed_tests: names.iter().map(|name| (*name).to_owned()).collect(),
            counts: failed.map(|failed| CheckCounts {
                failed,
                passed: Some(1),
                errors: 0,
                skipped: 0,
            }),
            ..Failures::default()
        }
    }

    /// The same tests in another order share a signature, worked from the hash of the sorted names;
    /// another test gives another. A list the counts do not vouch for gives none: no counts, a
    /// failed count above the names read (earlier failures cut from the output's tail) or below
    /// them, and no test named.
    #[test]
    fn only_a_whole_list_of_failing_tests_is_signed() {
        let one = failing_signature(&naming(&["t::b", "t::a"], Some(2)));
        assert_eq!(one, Some(ShortHash::of(b"t::a\nt::b")));
        assert_eq!(
            failing_signature(&naming(&["t::a", "t::b", "t::a"], Some(2))),
            one
        );
        assert_ne!(failing_signature(&naming(&["t::a"], Some(1))), one);
        assert_eq!(failing_signature(&naming(&["t::a", "t::b"], None)), None);
        assert_eq!(failing_signature(&naming(&["t::a", "t::b"], Some(3))), None);
        assert_eq!(failing_signature(&naming(&["t::a", "t::b"], Some(1))), None);
        assert_eq!(failing_signature(&naming(&[], Some(0))), None);
    }
}
