//! Fuzzes the IPC frame decoder: any bytes, read as a stream of frames the way the daemon reads a
//! client's requests and a client the daemon's answers, and decoded as one frame's body, must give
//! messages or an error: never a panic, a hang, or an allocation past the frame's size cap.

#![no_main]

use cortex_core::ipc::{Request, Response, decode, read_frame};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    // Each frame read takes its length prefix at least, so the reading ends.
    let mut requests = bytes;
    while read_frame::<Request>(&mut requests).is_ok() {}
    let mut responses = bytes;
    while read_frame::<Response>(&mut responses).is_ok() {}
    drop(decode::<Request>(bytes));
    drop(decode::<Response>(bytes));
});
