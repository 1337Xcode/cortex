//! Coordination across the sessions of one repository, of any agents: where each session is (what
//! it read, edited or claimed, and until when), which sessions an edit concerns, and what each is
//! told. Nothing here locks anything or sends anything: the daemon asks these rules and gives the
//! text in each agent's own hooks. The rules are generic over the session and node types, so
//! they are tested on hand-built values; the daemon uses the store's.

mod concern;
mod handoff;
mod notice;
mod position;

pub use concern::{Concern, EditAt, Link, Thresholds, Via, concerned};
pub use handoff::{Handed, MAX_HANDOFF_TOKENS, handoff_text};
pub use notice::{Party, to_editor, to_holder};
pub use position::{Position, PositionMode, Timing, is_live};
