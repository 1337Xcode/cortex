//! Joining an error to its causes, for a message a person reads.
//!
//! An error type here says what failed and carries what it failed because of as its source. A
//! message that shows only the first says too little ("writing a task"), and one that writes its
//! cause into its own text says it twice wherever the chain is shown as well. So the message
//! stays short and whoever prints it joins the chain.

use std::error::Error;
use std::fmt::Write as _;

/// How many causes are joined before the rest are left out: enough for the chains here, and a
/// bound in case one ever loops.
const MOST: usize = 8;

/// `error` and its causes, joined by `: `, as "writing a task: UNIQUE constraint failed".
///
/// A cause that repeats the text it came from is written once, since some libraries put their own
/// cause into their message as well and a reader gains nothing from the repeat.
#[must_use]
pub fn with_causes(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    let mut joined = 0;
    while let Some(cause) = source {
        if joined >= MOST {
            break;
        }
        let said = cause.to_string();
        if !text.ends_with(&said) {
            let _ = write!(text, ": {said}");
        }
        source = cause.source();
        joined += 1;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An error of three links reads as all three, in order, joined by `: `.
    #[test]
    fn an_error_reads_with_its_causes() {
        #[derive(Debug, thiserror::Error)]
        #[error("writing a task")]
        struct Outer(#[source] Middle);
        #[derive(Debug, thiserror::Error)]
        #[error("the statement failed")]
        struct Middle(#[source] Inner);
        #[derive(Debug, thiserror::Error)]
        #[error("UNIQUE constraint failed")]
        struct Inner;

        assert_eq!(
            with_causes(&Outer(Middle(Inner))),
            "writing a task: the statement failed: UNIQUE constraint failed"
        );
    }

    /// An error with nothing under it reads as itself.
    #[test]
    fn an_error_without_a_cause_reads_as_itself() {
        #[derive(Debug, thiserror::Error)]
        #[error("no database")]
        struct Alone;

        assert_eq!(with_causes(&Alone), "no database");
    }

    /// A cause already written into the text above it is not written again, which is what a
    /// library that puts its own cause in its message would otherwise make happen.
    #[test]
    fn a_cause_already_said_is_not_said_twice() {
        #[derive(Debug, thiserror::Error)]
        #[error("reading the schema: the file is damaged")]
        struct Outer(#[source] Inner);
        #[derive(Debug, thiserror::Error)]
        #[error("the file is damaged")]
        struct Inner;

        assert_eq!(
            with_causes(&Outer(Inner)),
            "reading the schema: the file is damaged"
        );
    }
}
