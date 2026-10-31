//! What Cortex learns from agents' work. For now, the record of it: [`Recorder`] turns each
//! normalized hook event into the session, task and event rows the store keeps.

#![warn(clippy::pedantic)]

mod record;

pub use record::Recorder;
