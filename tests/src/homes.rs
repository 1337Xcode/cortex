//! Home folders written in text, in every spelling a captured payload can carry.
//!
//! Text is split into path tokens at spaces, quotes, brackets and the characters that separate
//! list items, assignments and drive letters (`,`, `;`, `=`, `|`, `:`). A token that starts with a
//! separator is an absolute path. Its first `users` or `home` folder (any case) is a home folder
//! when every folder before it is a known root: a drive (`c` from Git Bash, `c$` from an admin
//! share, `c%3a` from a file address), `?` from a verbatim path, `mnt`, `cygdrive`, `var`, `usr`,
//! `export`, `localhost`, `wsl$` or `wsl.localhost` and the distribution after it. `..` folds into
//! the folder before it. The folder after the container is the user name.
//! This finds `C:\Users\name`, `\\?\C:\Users\name`, `/c/Users/name`, `/mnt/c/Users/name`,
//! `/Users/name`, `/home/name`, `/var/home/name`, `\\wsl$\Ubuntu\home\name` and escaped forms such
//! as `\/home\/name`, and leaves out relative paths, project folders under a repository and web
//! addresses. A `/users/name` written at the start of a token is read as a home folder even when it
//! is a web route; a path broken by a space (`/Volumes/Macintosh HD/Users/name`) is not followed.
//!
//! Claude Code names a transcript folder after the project path with every other character
//! replaced by `-` (`C--Users-name-repo`, `-mnt-c-Users-name-repo`, `-home-name-repo`,
//! `--wsl--Ubuntu-home-name-repo`). A token of letters, digits and hyphens whose first `users` or
//! `home` follows nothing but known roots is reported whatever follows, when it starts like a
//! path (see [`is_encoded_home`]): the scrubber collapses every encoded project folder to
//! `C--repo`, so none may remain. Punctuation that prose puts after a name is ignored.

/// Folder names under a home root that belong to no one user: shared and default profiles,
/// Homebrew's shared prefix on Linux, and the scrubbed user.
pub const SHARED_HOME_NAMES: &[&str] = &[
    crate::scrub::SCRUBBED_USER,
    "public",
    "default",
    "shared",
    "all",
    "linuxbrew",
];

/// The folder names that hold user homes.
const HOME_CONTAINERS: &[&str] = &["users", "home"];

/// Folder names that may sit above a home container, lower case.
const ROOT_FOLDERS: &[&str] = &[
    "?",
    "mnt",
    "cygdrive",
    "var",
    "usr",
    "export",
    "localhost",
    "wsl$",
    "wsl.localhost",
    "wsl",
];

/// Folders whose next folder is a WSL distribution or share name, which may sit above `home` or
/// `Users`. The encoded forms of `wsl$` and `wsl.localhost` are `wsl` and `wsl`, `localhost`.
const DISTRIBUTION_HOSTS: &[&str] = &["wsl$", "wsl.localhost", "wsl", "localhost"];

/// Characters prose may put right after an encoded folder name, which are not part of it.
const TRAILING_PUNCTUATION: &[char] = &['.', '!', '?'];

/// Characters that end a path token.
const TOKEN_ENDS: &[char] = &[
    '"', '\'', '`', '(', ')', '[', ']', '{', '}', '<', '>', ',', ';', '=', '|', ':',
];

/// Path separators, either of which may repeat or mix.
const SEPARATORS: &[char] = &['/', '\\'];

/// The user name of every home folder in `text` that is not a shared or scrubbed one.
#[must_use]
pub fn home_users_in(text: &str) -> Vec<String> {
    tokens(text)
        .filter(|token| token.starts_with(SEPARATORS))
        .filter_map(home_user)
        .filter(|user| !SHARED_HOME_NAMES.contains(&user.as_str()))
        .collect()
}

/// Every hyphen-encoded transcript folder in `text` that names a home, lower case.
#[must_use]
pub fn encoded_homes_in(text: &str) -> Vec<String> {
    tokens(text)
        .flat_map(|token| token.split(SEPARATORS))
        .map(str::to_ascii_lowercase)
        .filter(|token| is_encoded_home(token))
        .collect()
}

/// The path tokens of `text`.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| character.is_whitespace() || TOKEN_ENDS.contains(&character))
        .filter(|token| !token.is_empty())
}

/// The user name after the first home container of an absolute path token, when every folder
/// before the container is a known root.
fn home_user(token: &str) -> Option<String> {
    let mut folders: Vec<String> = Vec::new();
    for folder in token.split(SEPARATORS) {
        match folder {
            "" | "." => {}
            ".." => {
                folders.pop();
            }
            name => folders.push(name.to_ascii_lowercase()),
        }
    }
    let container = folders
        .iter()
        .position(|folder| HOME_CONTAINERS.contains(&folder.as_str()))?;
    let is_rooted = folders
        .get(..container)?
        .iter()
        .enumerate()
        .all(|(index, folder)| {
            let previous = index.checked_sub(1).and_then(|before| folders.get(before));
            is_root_folder(folder, previous.map(String::as_str))
        });
    if !is_rooted {
        return None;
    }
    let user: String = folders
        .get(container + 1)?
        .chars()
        .take_while(|character| character.is_alphanumeric() || "._-".contains(*character))
        .collect();
    (!user.is_empty()).then_some(user)
}

/// Whether a lower-case folder may sit above a home container: a known root, a drive letter
/// (`c`, `c$`, or `c%3a` as a file address writes `c:`), or the name after a WSL host or share.
fn is_root_folder(folder: &str, previous: Option<&str>) -> bool {
    let mut characters = folder.chars();
    let first = characters.next();
    let rest = characters.as_str();
    let is_drive = first.is_some_and(|letter| letter.is_ascii_alphabetic())
        && (rest.is_empty() || rest == "$" || rest == "%3a");
    is_drive
        || ROOT_FOLDERS.contains(&folder)
        || previous.is_some_and(|host| DISTRIBUTION_HOSTS.contains(&host))
}

/// Whether a lower-case token is an encoded transcript folder that names a home: letters, digits
/// and hyphens, with its first home container after nothing but known roots. A token without a
/// leading hyphen must start with a drive and two hyphens (`c--users-`); one leading hyphen starts
/// a Unix path (`-home-`, `-mnt-c-users-`); two or more start a share or a verbatim path
/// (`--wsl--ubuntu-home-`, `----c--users-`) only when a root comes before the container, since
/// `--home-dir` is a flag.
fn is_encoded_home(token: &str) -> bool {
    let token = token.trim_end_matches(TRAILING_PUNCTUATION);
    let is_encoded = !token.is_empty()
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if !is_encoded {
        return false;
    }
    let folders: Vec<&str> = token
        .split('-')
        .filter(|folder| !folder.is_empty())
        .collect();
    let Some(container) = folders
        .iter()
        .position(|folder| HOME_CONTAINERS.contains(folder))
    else {
        return false;
    };
    let is_rooted = folders.get(..container).is_some_and(|roots| {
        roots.iter().enumerate().all(|(position, folder)| {
            let previous = position
                .checked_sub(1)
                .and_then(|at| roots.get(at))
                .copied();
            is_root_folder(folder, previous)
        })
    });
    let leading_hyphens = token.len() - token.trim_start_matches('-').len();
    let starts_like_a_path = match leading_hyphens {
        0 => {
            token.get(1..3) == Some("--")
                && token
                    .chars()
                    .next()
                    .is_some_and(|letter| letter.is_ascii_alphabetic())
        }
        1 => true,
        _ => container > 0,
    };
    starts_like_a_path && is_rooted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each spelling of a real home folder gives its user, worked out by hand.
    #[test]
    fn absolute_homes_give_their_user() {
        let cases = [
            ("C:\\Users\\alice\\x", "alice"),
            ("c:\\users\\alice", "alice"),
            ("\\\\?\\C:\\Users\\alice\\x", "alice"),
            ("C:\\\\Users\\\\alice\\\\x", "alice"),
            ("C:/Users\\alice/x", "alice"),
            ("/c/Users/alice/x", "alice"),
            ("/mnt/c/Users/alice/x", "alice"),
            ("/cygdrive/c/Users/alice", "alice"),
            ("\\\\localhost\\c$\\Users\\alice\\x", "alice"),
            ("/Users/alice/proj", "alice"),
            ("/USERS/alice", "alice"),
            ("/home/alice/project", "alice"),
            ("/var/home/alice/repo", "alice"),
            ("\\\\wsl$\\Ubuntu\\home\\alice\\repo", "alice"),
            ("\\\\wsl.localhost\\Ubuntu\\home\\alice\\repo", "alice"),
            ("{\"p\":\"\\/home\\/alice\\/x\"}", "alice"),
            ("file:///home/alice/x", "alice"),
            (
                "{\"uri\":\"file:///c%3A/Users/alice/proj/src/x.ts\"}",
                "alice",
            ),
            (r"C:\Users\dev\..\alice\x", "alice"),
            ("https://example.com,/home/alice/x", "alice"),
        ];
        for (text, user) in cases {
            assert_eq!(home_users_in(text), vec![user.to_owned()], "{text}");
        }
    }

    /// Shared profiles, relative paths, project folders and web addresses give nothing.
    #[test]
    fn other_paths_give_nothing() {
        let cases = [
            "C:\\Users\\dev\\.codex\\x.jsonl and /home/dev/x and /c/Users/dev",
            "C:/Users/Public/Desktop and /Users/Shared/x and /home/linuxbrew/.linuxbrew/bin",
            "src/routes/users/profile.ts and app/home/page.tsx and ~/projects/home/x",
            "src/a/users/bob.ts and ui/src/v/home/page.tsx",
            "C:\\repo\\src\\users\\profile.ts and /repo/app/home/page.tsx",
            "/opt/app/users/list.json",
            "https://api.github.com/users/octocat and https://example.com/a/users/bob",
            "PATH=/usr/bin:/home/dev/bin",
            "the home folder of users",
        ];
        for text in cases {
            assert_eq!(home_users_in(text), Vec::<String>::new(), "{text}");
        }
    }

    /// Encoded transcript folders are found under any known root and after any separator; flags,
    /// ordinary hyphenated words and relative encodings are not.
    #[test]
    fn encoded_homes_are_found() {
        let found = [
            "projects\\C--Users-alice-Desktop-repo\\x.jsonl",
            "projects\\C--Users-Dev-Smith-Desktop-x\\y.jsonl",
            "projects/-home-alice-repo/x.jsonl",
            "C:\\Users\\dev\\.claude\\projects\\-mnt-c-Users-alice-repo\\x.jsonl",
            "-var-home-alice-repo",
            "dir=-Users-alice-proj",
            "pass the -home-dir flag",
            r"projects\--wsl--Ubuntu-home-alice-repo\x.jsonl",
            "--wsl-localhost-Ubuntu-home-alice-repo",
            "----C--Users-alice-repo",
            "see C--Users-alice-repo.",
        ];
        for text in found {
            assert_eq!(encoded_homes_in(text).len(), 1, "{text}");
        }
        let quiet = [
            "C:\\Users\\dev\\.claude\\projects\\C--repo\\x.jsonl",
            "take-home-assignment.md, a work-from-home policy, the --home-dir flag",
            "-src-routes-users-bob",
        ];
        for text in quiet {
            assert_eq!(encoded_homes_in(text), Vec::<String>::new(), "{text}");
        }
    }
}
