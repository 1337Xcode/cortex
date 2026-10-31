//! How the hook, the CLI and a repository's daemon talk: framed JSON messages, a handshake that
//! checks versions first, and the versions themselves.
//!
//! Only the message format lives here. Each side does its own reading and writing: the hook and
//! the CLI block on a local socket or named pipe, the daemon reads asynchronously; both encode and
//! decode through this module, so the format has one implementation.

mod frame;
mod message;
mod version;

pub use self::frame::{
    FrameError, LENGTH_PREFIX_BYTES, MAX_FRAME_BYTES, body_len, decode, encode, read_frame,
    write_frame,
};
pub use self::message::{
    Hello, Mismatch, Request, RequestBody, Response, ResponseBody, Role, StatusReport, Welcome,
};
pub use self::version::{CORTEX_VERSION, CortexVersion, Protocol, VersionError};

#[cfg(test)]
mod tests;
