//! The interface contracts behind the coordination notices queued lately: for a notice that one
//! session changed the signature of definitions a file another session holds calls, what the
//! signatures were and became, kept in memory until the concerned session's next hook renders the
//! notice. The store keeps the notice in the words that name the definitions alone, since the
//! signatures are code and code never reaches the store; a notice rendered without its contract,
//! after a restart or once it was pushed out of here, is given in those words.
//!
//! Beside them are the definitions of the edited files as read lately, by the file's path and the
//! hash of its bytes, so that checking a contract again while the file is unchanged costs a read
//! and a hash of the file, never another parse.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cortex_coord::{Holding, Signature};
use cortex_core::{ContentHash, Identifier, NoticeSubject, RepoPath, ShortHash};
use cortex_store::SymbolId;

/// The most contracts kept, the oldest pushed out first.
const MAX_KEPT: usize = 256;

/// The most files whose definitions are kept as read, the oldest pushed out first.
const MAX_READ: usize = 32;

/// A definition whose signature an edit changed, with its signature before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Changed {
    /// The stored symbol.
    pub(crate) symbol: SymbolId,
    /// Its qualified name, by which the edited file is read again.
    pub(crate) qual_name: Identifier,
    /// Its name, as its calls write it.
    pub(crate) name: Identifier,
    /// Its signature before the edit, as the index read it.
    pub(crate) before: Signature,
    /// Its signature after the edit, as the file on disk read.
    pub(crate) after: Signature,
    /// The hash of its header after the edit, which the file on disk must still give when the
    /// notice is rendered.
    pub(crate) after_hash: ShortHash,
}

/// Which queued notice a contract belongs to: the session it is for, what it is about, and when it
/// was made, in UTC milliseconds, as the store keeps them.
pub(crate) type NoticeKey = (Identifier, NoticeSubject, i64);

/// The contract behind one queued notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Contract {
    /// The notice it belongs to.
    pub(crate) key: NoticeKey,
    /// How the concerned session holds its file.
    pub(crate) holding: Holding,
    /// The editing agent, as the notice names it.
    pub(crate) editor: String,
    /// The working tree the edit was made in.
    pub(crate) edited_root: PathBuf,
    /// The edited file.
    pub(crate) edited: RepoPath,
    /// The file the concerned session holds, which calls what changed.
    pub(crate) held: RepoPath,
    /// The definitions the edit changed that the held file called when the notice was queued.
    pub(crate) changed: Vec<Changed>,
}

/// A file's definitions as read: each qualified name that one definition alone has, with the hash
/// of its header.
pub(crate) type Definitions = Arc<Vec<(Identifier, ShortHash)>>;

/// One file's definitions, as read from its bytes of one hash.
type Read = (RepoPath, ContentHash, Definitions);

/// The contracts of the notices queued lately, and the edited files' definitions as read lately.
#[derive(Debug, Default)]
pub(crate) struct Contracts {
    /// The contracts, oldest first.
    kept: Mutex<VecDeque<Contract>>,
    /// The definitions read, oldest first.
    read: Mutex<VecDeque<Read>>,
}

impl Contracts {
    /// Keeps `contract`, pushing out the oldest past [`MAX_KEPT`].
    pub(crate) fn keep(&self, contract: Contract) {
        let mut kept = locked(&self.kept);
        if kept.len() == MAX_KEPT {
            kept.pop_front();
        }
        kept.push_back(contract);
    }

    /// The contract of the notice `key` names, if one is kept.
    pub(crate) fn of(&self, key: &NoticeKey) -> Option<Contract> {
        locked(&self.kept)
            .iter()
            .rev()
            .find(|contract| contract.key == *key)
            .cloned()
    }

    /// Keeps `definitions` as what the file at `path` holding bytes of hash `hash` defines,
    /// pushing out the oldest past [`MAX_READ`].
    pub(crate) fn keep_read(&self, path: &RepoPath, hash: ContentHash, definitions: Definitions) {
        let mut read = locked(&self.read);
        read.retain(|(kept, _, _)| kept != path);
        if read.len() == MAX_READ {
            read.pop_front();
        }
        read.push_back((path.clone(), hash, definitions));
    }

    /// What the file at `path` defines when it holds bytes of hash `hash`, if it was read so.
    pub(crate) fn read_of(&self, path: &RepoPath, hash: ContentHash) -> Option<Definitions> {
        locked(&self.read)
            .iter()
            .find(|(kept, kept_hash, _)| kept == path && *kept_hash == hash)
            .map(|(_, _, definitions)| Arc::clone(definitions))
    }
}

/// `mutex`, locked. Each change to what it holds is made whole before it is let go, so a poisoned
/// lock is taken as it is.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
