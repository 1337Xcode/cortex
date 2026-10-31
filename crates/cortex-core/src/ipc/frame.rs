//! Frames: each message travels as a 4-byte little-endian length, then that many bytes of JSON.

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Most bytes a frame's body may hold: 1 MiB. A hook payload is read with the same cap, so any
/// event fits.
pub const MAX_FRAME_BYTES: usize = 1_048_576;

/// Bytes in a frame's length prefix.
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// Why a frame could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// Reading or writing the connection failed, or it closed in the middle of a frame.
    #[error("the connection failed")]
    Io(#[from] io::Error),
    /// The body would be, or is announced as, longer than [`MAX_FRAME_BYTES`].
    #[error("a frame body of {len_bytes} bytes is longer than {MAX_FRAME_BYTES}")]
    TooLarge {
        /// The body's length in bytes.
        len_bytes: usize,
    },
    /// The body is not the JSON of the message expected.
    #[error("a frame body is not the message expected")]
    Json(#[from] serde_json::Error),
}

/// The frame holding `message`: its JSON's length, then the JSON.
///
/// # Errors
///
/// The message cannot be written as JSON, or its JSON is longer than [`MAX_FRAME_BYTES`].
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(message)?;
    let len = u32::try_from(body.len())
        .ok()
        .filter(|_| body.len() <= MAX_FRAME_BYTES)
        .ok_or(FrameError::TooLarge {
            len_bytes: body.len(),
        })?;
    let mut frame = Vec::with_capacity(LENGTH_PREFIX_BYTES + body.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// The body length a frame's prefix announces.
///
/// # Errors
///
/// The length is over [`MAX_FRAME_BYTES`]; nothing more of the connection should be read.
pub fn body_len(prefix: [u8; LENGTH_PREFIX_BYTES]) -> Result<usize, FrameError> {
    let len_bytes = usize::try_from(u32::from_le_bytes(prefix)).unwrap_or(usize::MAX);
    if len_bytes > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge { len_bytes });
    }
    Ok(len_bytes)
}

/// The message a frame's body holds.
///
/// # Errors
///
/// The body is not the JSON of a `T`.
pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    Ok(serde_json::from_slice(body)?)
}

/// Reads one frame from `reader` and the message it holds.
///
/// # Errors
///
/// The reader fails or ends before the frame does, the length is over the cap, or the body is not
/// a `T`.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T, FrameError> {
    let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
    reader.read_exact(&mut prefix)?;
    let mut body = vec![0u8; body_len(prefix)?];
    reader.read_exact(&mut body)?;
    decode(&body)
}

/// Writes `message` to `writer` as one frame.
///
/// # Errors
///
/// The message cannot be encoded (see [`encode`]) or the writer fails.
pub fn write_frame<T: Serialize>(writer: &mut impl Write, message: &T) -> Result<(), FrameError> {
    writer.write_all(&encode(message)?)?;
    writer.flush()?;
    Ok(())
}
