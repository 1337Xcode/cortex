//! What can go wrong reading a transcript.

/// A transcript that could not be read. A line that cannot be understood is not an error: it is
/// passed over, as a line that reports no response is.
#[derive(Debug, thiserror::Error)]
pub enum MeterError {
    /// The file could not be measured, moved to the place reading starts, or read.
    #[error("reading a transcript: {0}")]
    Read(#[from] std::io::Error),
}
