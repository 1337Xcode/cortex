//! Why a file is listed, in words a person can check: the strongest true reason of those the
//! retrieval's evidence holds, at most 60 characters.
//!
//! The reasons, strongest first: the prompt named the file or a symbol of it; the session edited
//! it; the prompt's words are in its names; it changed with another file the evidence points at, in
//! so many of its commits; it refers to such a file, or such a file refers to it, by a reference
//! tied at 0.8 or more; tasks like this one changed it; agents worked on it with such a file
//! before; the session read it. A file none of these holds is not listed: a reason is never made
//! up.

use cortex_core::ReasonKind;

/// Most characters a reason may hold.
pub const MAX_REASON_CHARS: usize = 60;

/// Most of the prompt's words a reason names.
const MAX_WORDS_NAMED: usize = 3;

/// Why a file is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The prompt names it, or a symbol in it, exactly.
    Named,
    /// The session edited it.
    EditedHere,
    /// The prompt's words it holds, in the prompt's order.
    Matches(Vec<String>),
    /// The work tree holds it changed and not committed.
    Uncommitted,
    /// It changed in `together` of its `of` commits with the file named `other`.
    ChangesWith {
        /// The other file's name.
        other: String,
        /// The commits that changed both.
        together: u32,
        /// The commits that changed it.
        of: u32,
    },
    /// Code in it refers to the file named here, by a reference tied at 0.8 or more.
    RefersTo(String),
    /// Code in the file named here refers to it, by a reference tied at 0.8 or more.
    ReferredToBy(String),
    /// Earlier tasks like this one changed it.
    LikeEarlierWork,
    /// Agents worked on it together with the file named here before.
    WorkedOnWith(String),
    /// The session read it.
    ReadHere,
}

impl Reason {
    /// What kind of reason it is, without the words it says it in.
    #[must_use]
    pub fn kind(&self) -> ReasonKind {
        match self {
            Self::Named => ReasonKind::Named,
            Self::EditedHere => ReasonKind::EditedHere,
            Self::Matches(_) => ReasonKind::Matches,
            Self::Uncommitted => ReasonKind::Uncommitted,
            Self::ChangesWith { .. } => ReasonKind::ChangesWith,
            Self::RefersTo(_) => ReasonKind::RefersTo,
            Self::ReferredToBy(_) => ReasonKind::ReferredToBy,
            Self::LikeEarlierWork => ReasonKind::LikeEarlierWork,
            Self::WorkedOnWith(_) => ReasonKind::WorkedOnWith,
            Self::ReadHere => ReasonKind::ReadHere,
        }
    }

    /// The reason as the work order writes it, at most [`MAX_REASON_CHARS`] characters: a name too
    /// long to fit is cut short with `...`.
    #[must_use]
    pub fn text(&self) -> String {
        let text = match self {
            Self::Named => "named in the prompt".to_owned(),
            Self::EditedHere => "edited earlier in this session".to_owned(),
            Self::Matches(words) if words.is_empty() => "matches the prompt".to_owned(),
            Self::Matches(words) => {
                let named: Vec<&str> = words
                    .iter()
                    .take(MAX_WORDS_NAMED)
                    .map(String::as_str)
                    .collect();
                format!("matches {}", named.join(", "))
            }
            Self::Uncommitted => "an uncommitted change".to_owned(),
            Self::ChangesWith {
                other,
                together,
                of,
            } => {
                let tail = format!(" in {together} of {of} commits");
                format!(
                    "changes with {}{tail}",
                    fitted(other, "changes with ".len() + tail.len())
                )
            }
            Self::RefersTo(other) => format!("refers to {}", fitted(other, 10)),
            Self::ReferredToBy(other) => format!("referred to by {}", fitted(other, 15)),
            Self::LikeEarlierWork => "changed in similar earlier tasks".to_owned(),
            Self::WorkedOnWith(other) => format!("worked on with {} before", fitted(other, 22)),
            Self::ReadHere => "read earlier in this session".to_owned(),
        };
        fitted(&text, 0)
    }
}

/// `name` cut so that it fits in what [`MAX_REASON_CHARS`] leaves after `around` other characters,
/// its end replaced with `...` when cut.
fn fitted(name: &str, around: usize) -> String {
    let room = MAX_REASON_CHARS.saturating_sub(around);
    if name.chars().count() <= room {
        return name.to_owned();
    }
    let kept: String = name.chars().take(room.saturating_sub(3)).collect();
    format!("{kept}...")
}

/// What the retrieval's evidence holds about one file, gathered for its reason.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate fact a reason may rest on, not a state of one thing"
)]
pub struct ItemEvidence {
    /// The prompt names it, or a symbol in it, exactly.
    pub is_named: bool,
    /// The session edited it.
    pub is_edited_here: bool,
    /// The prompt's words it holds, in the prompt's order.
    pub words: Vec<String>,
    /// The work tree holds it changed and not committed.
    pub is_uncommitted: bool,
    /// Another file the evidence points at that it changes with: its name, the commits that changed
    /// both and the commits that changed this one.
    pub changes_with: Option<(String, u32, u32)>,
    /// Another such file its code refers to, by a reference tied at 0.8 or more.
    pub refers_to: Option<String>,
    /// Another such file whose code refers to it, by a reference tied at 0.8 or more.
    pub referred_to_by: Option<String>,
    /// Earlier tasks like this one changed it.
    pub is_like_earlier_work: bool,
    /// Another such file agents worked on together with it before.
    pub worked_on_with: Option<String>,
    /// The session read it.
    pub is_read_here: bool,
}

/// The strongest reason `evidence` holds, in the order [`Reason`] lists them; none when it holds
/// none, and the file is then not listed.
#[must_use]
pub fn reason_of(evidence: &ItemEvidence) -> Option<Reason> {
    if evidence.is_named {
        return Some(Reason::Named);
    }
    if evidence.is_edited_here {
        return Some(Reason::EditedHere);
    }
    if !evidence.words.is_empty() {
        return Some(Reason::Matches(evidence.words.clone()));
    }
    if evidence.is_uncommitted {
        return Some(Reason::Uncommitted);
    }
    if let Some((other, together, of)) = &evidence.changes_with {
        return Some(Reason::ChangesWith {
            other: other.clone(),
            together: *together,
            of: *of,
        });
    }
    if let Some(other) = &evidence.refers_to {
        return Some(Reason::RefersTo(other.clone()));
    }
    if let Some(other) = &evidence.referred_to_by {
        return Some(Reason::ReferredToBy(other.clone()));
    }
    if evidence.is_like_earlier_work {
        return Some(Reason::LikeEarlierWork);
    }
    if let Some(other) = &evidence.worked_on_with {
        return Some(Reason::WorkedOnWith(other.clone()));
    }
    evidence.is_read_here.then_some(Reason::ReadHere)
}
