//! Coordination across the sessions of one repository, of any agents: where each session is (what
//! it read, edited or claimed, and until when), which sessions an edit concerns, and what each is
//! told. Nothing here locks anything or sends anything: the daemon asks these rules and gives the
//! text in each agent's own hooks. The rules are generic over the session and node types, so
//! they are tested on hand-built values; the daemon uses the store's.

mod concern;
mod contract;
mod handoff;
mod notice;
mod position;
mod signature;

pub use concern::{Concern, EditAt, Link, Thresholds, Via, concerned};
pub use contract::{Called, Holding, contract_to_holder};
pub use handoff::{FailedCheck, Handed, MAX_HANDOFF_TOKENS, ago, handoff_text};
pub use notice::{Notice, Party, to_editor, to_holder};
pub use position::{Position, PositionMode, Timing, is_live};
pub use signature::{MAX_SIGNATURE_CHARS, Signature};
