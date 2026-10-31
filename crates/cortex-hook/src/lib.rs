//! The hook client: what runs when an agent calls `cortex hook`, and the one client of the
//! daemon's protocol, which the CLI uses too.
//!
//! This crate depends on `cortex-core` alone among Cortex's crates, so that a hook starts without
//! loading anything it does not need.

#![warn(clippy::pedantic)]

mod adapter;
mod claude;
mod client;

pub use adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, adapter};
pub use client::{Answer, ClientError, ask};
