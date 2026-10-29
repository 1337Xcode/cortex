//! What a scrubbed fixture may contain.
//!
//! A captured hook payload carries the capturing machine's paths, user name, machine name and live
//! ids. The scrubber replaces them with fixed values before a fixture is committed; these checks
//! find anything it missed, in every key and value of a payload and in the text of a fixture
//! README.

/// Folders a scrubbed `cwd` may be, or be inside.
pub const SCRUBBED_REPO_ROOTS: &[&str] = &["C:\\repo", "/repo"];

/// Folder prefixes a scrubbed transcript path may start with, including the Windows verbatim form
/// (`\\?\`) Codex writes when it resumes a session.
pub const SCRUBBED_HOMES: &[&str] = &["C:\\Users\\dev\\", "\\\\?\\C:\\Users\\dev\\", "/home/dev/"];

/// The only user name a scrubbed home folder may show.
pub const SCRUBBED_USER: &str = "dev";

/// Prefix of every scrubbed UUID-style id.
pub const SCRUBBED_UUID_PREFIX: &str = "00000000-0000-4000-8000-";

/// Payload fields that hold a transcript path, which must sit under a scrubbed home.
const TRANSCRIPT_KEYS: &[&str] = &["transcript_path", "agent_transcript_path"];

/// Folder names under a home root that belong to no one user: shared and default profiles, and
/// the scrubbed user.
const SHARED_HOME_NAMES: &[&str] = &[SCRUBBED_USER, "public", "default", "shared", "all"];

/// Lower-case markers of the hyphen-encoded folder names Claude Code gives transcript folders
/// (`C--Users-<name>-...` on Windows, `-Users-<name>-...` on macOS, `-home-<name>-...` on Linux).
/// The scrubber collapses a whole encoded folder to `C--repo`, and the name cannot be told apart
/// from the path after it, so every encoded home folder is reported.
const ENCODED_MARKERS: &[&str] = &["-users-", "-home-"];

/// Characters that may come right before an encoded folder name: a path separator, a quote, an
/// assignment or a list separator. A space does not count, so a flag such as `-home-dir` in prose
/// is not read as a folder.
const ENCODED_FOLDER_STARTS: &[char] = &['/', '\\', '"', '\'', '`', '=', ':', '(', ','];

/// The names Windows gives a new machine, as a lower-case prefix and the length of the random
/// part after it: `DESKTOP-7F3KQ2M`, `LAPTOP-7F3KQ2M`, and `WIN-` with eleven on servers.
const MACHINE_NAMES: &[(&str, usize)] = &[("desktop-", 7), ("laptop-", 7), ("win-", 11)];

/// Keys whose values are session, prompt, turn, subagent or tool-call ids.
const ID_KEYS: &[&str] = &[
    "session_id",
    "prompt_id",
    "turn_id",
    "agent_id",
    "agentId",
    "tool_use_id",
];

/// Keys whose values are opaque hashes of content the capture cannot inspect; the scrubber
/// replaces every character with `0`.
const ZEROED_KEYS: &[&str] = &["harnessSectionHash"];

/// The forms a scrubbed id takes.
const FIXED_ID_PREFIXES: &[&str] = &[
    SCRUBBED_UUID_PREFIX,
    "exec-00000000-0000-4000-8000-",
    "toolu_fixture",
    "call_fixture",
    "fixtureagent",
];

/// Ids and tokens that also appear inside text: the prefix, the fixed form, and the shortest run
/// of letters and digits after the prefix that counts as an id. Codex call ids are 32 hex digits;
/// a shorter run after `call_` is an ordinary word such as `call_sites`. Anthropic message and
/// request ids start `msg_` and `req_`; Codex passes a subagent's context as an encrypted Fernet
/// token, which starts `gAAAAA`.
const EMBEDDED_IDS: &[(&str, &str, usize)] = &[
    ("toolu_", "toolu_fixture", 1),
    ("call_", "call_fixture", 16),
    ("msg_", "msg_fixture", 16),
    ("req_", "req_fixture", 16),
    ("gAAAAA", "gAAAAA_fixture", 16),
];

/// Length of a UUID in its hyphenated text form: `8-4-4-4-12` hexadecimal digits.
const UUID_LEN: usize = 36;

/// Path separators, either of which may repeat (`C:\\Users`, `\/home\/` in escaped JSON).
const SEPARATORS: &[char] = &['/', '\\'];

/// Where the folder before a home folder name sits in its path.
#[derive(Debug, PartialEq, Eq)]
enum PathRoot {
    /// After a drive: `C:\`, `\\?\C:\`, or `/c/` and `/mnt/c/` from Git Bash and WSL.
    Drive,
    /// At the top of an absolute path: `/home`, `/Users`.
    Top,
    /// Inside a relative path, such as `src/routes/users/`.
    Relative,
}

/// Every problem with one parsed payload: its folder and session fields, every id-valued and
/// hash-valued key, and the text of every key and string value.
#[must_use]
pub fn payload_problems(payload: &serde_json::Value) -> Vec<String> {
    let mut problems = Vec::new();
    let field = |key: &str| payload.get(key).and_then(serde_json::Value::as_str);
    if let Some(cwd) = field("cwd")
        && !SCRUBBED_REPO_ROOTS.iter().any(|root| is_inside(cwd, root))
    {
        problems.push(format!("cwd {cwd:?} is not a scrubbed repository path"));
    }
    for key in TRANSCRIPT_KEYS {
        if let Some(transcript) = field(key)
            && !SCRUBBED_HOMES
                .iter()
                .any(|home| transcript.starts_with(home))
        {
            problems.push(format!("{key} {transcript:?} is not under a scrubbed home"));
        }
    }
    let mut pairs = Vec::new();
    key_values(payload, &mut pairs);
    for (key, value) in &pairs {
        problems.extend(value_problems(key, value));
    }
    let mut texts = Vec::new();
    strings_in(payload, &mut texts);
    problems.extend(texts.into_iter().flat_map(text_problems));
    problems
}

/// Every unscrubbed id, home folder, machine name or email address in one piece of text.
#[must_use]
pub fn text_problems(text: &str) -> Vec<String> {
    // ASCII lowering keeps every byte offset, so positions found in `lowered` hold in `text`.
    let lowered = text.to_ascii_lowercase();
    let mut problems: Vec<String> = uuids_in(text)
        .into_iter()
        .filter(|id| !id.starts_with(SCRUBBED_UUID_PREFIX))
        .map(|id| format!("id {id} is not a fixed id"))
        .collect();
    problems.extend(
        embedded_ids_in(text)
            .into_iter()
            .map(|id| format!("id {id} is not a fixed id")),
    );
    problems.extend(
        home_users_in(text, &lowered)
            .into_iter()
            .map(|user| format!("home folder of user {user:?} is not scrubbed")),
    );
    problems.extend(
        encoded_homes_in(&lowered)
            .into_iter()
            .map(|folder| format!("encoded home folder {folder:?} is not scrubbed")),
    );
    problems.extend(
        machine_names_in(text, &lowered)
            .into_iter()
            .map(|name| format!("machine name {name:?}")),
    );
    problems.extend(
        emails_in(text)
            .into_iter()
            .map(|email| format!("email address {email:?}")),
    );
    problems
}

/// The problems with one key and its value: an id that is not in a fixed form or not a string,
/// or an opaque hash that is not zeroed.
fn value_problems(key: &str, value: &serde_json::Value) -> Vec<String> {
    let mut problems = Vec::new();
    if ID_KEYS.contains(&key) {
        match value {
            serde_json::Value::Null => {}
            serde_json::Value::String(id) => {
                if !id.is_empty()
                    && !FIXED_ID_PREFIXES
                        .iter()
                        .any(|prefix| id.starts_with(prefix))
                {
                    problems.push(format!("{key} {id:?} is not a fixed id"));
                }
            }
            other => problems.push(format!("{key} {other} is not a string id")),
        }
    }
    if ZEROED_KEYS.contains(&key)
        && !value
            .as_str()
            .is_some_and(|hash| hash.chars().all(|character| character == '0'))
    {
        problems.push(format!("{key} {value} is not zeroed"));
    }
    problems
}

/// Whether `path` is `root` or a path inside it.
fn is_inside(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(SEPARATORS))
}

/// Every key and every string value inside `value`, depth first.
fn strings_in<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(text) => out.push(text),
        serde_json::Value::Array(items) => items.iter().for_each(|item| strings_in(item, out)),
        serde_json::Value::Object(fields) => {
            for (key, field) in fields {
                out.push(key);
                strings_in(field, out);
            }
        }
        _ => {}
    }
}

/// Every key with its value, at any depth.
fn key_values<'a>(value: &'a serde_json::Value, out: &mut Vec<(&'a str, &'a serde_json::Value)>) {
    match value {
        serde_json::Value::Array(items) => items.iter().for_each(|item| key_values(item, out)),
        serde_json::Value::Object(fields) => {
            for (key, field) in fields {
                out.push((key.as_str(), field));
                key_values(field, out);
            }
        }
        _ => {}
    }
}

/// Whether `bytes` is a UUID in its hyphenated `8-4-4-4-12` hexadecimal form.
fn is_uuid(bytes: &[u8]) -> bool {
    bytes.len() == UUID_LEN
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Every UUID-shaped run of characters in `text`, in order.
fn uuids_in(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    (0..=bytes.len().saturating_sub(UUID_LEN))
        .filter(|&start| bytes.get(start..start + UUID_LEN).is_some_and(is_uuid))
        .filter_map(|start| text.get(start..start + UUID_LEN))
        .collect()
}

/// Every embedded id or token that is not in its fixed form.
fn embedded_ids_in(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for (prefix, fixed, min_len) in EMBEDDED_IDS {
        for (index, _) in text.match_indices(prefix) {
            let rest = text.get(index + prefix.len()..).unwrap_or_default();
            let run: String = rest
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            let id = format!("{prefix}{run}");
            if run.len() >= *min_len && !id.starts_with(fixed) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Whether a character can be part of a user name as it appears in a path.
fn is_name_char(character: char) -> bool {
    character.is_alphanumeric() || "._-".contains(character)
}

/// Whether a character can be part of a folder name, as opposed to what starts a path: a space,
/// a quote, `=`, `:`, a comma or a bracket.
fn is_segment_char(character: char) -> bool {
    character.is_alphanumeric() || "._-~%$+@".contains(character)
}

/// Where a path that continues after `head` starts: after a drive, at the top, or inside a
/// relative path. `head` is the text before the separators that precede a folder name.
fn path_root(head: &str) -> PathRoot {
    let mut back = head.chars().rev();
    let (last, second, third) = (back.next(), back.next(), back.next());
    let is_word_start =
        |character: Option<char>| character.is_none_or(|before| !before.is_alphanumeric());
    match (last, second) {
        // `C:` with nothing alphanumeric before the letter.
        (Some(':'), Some(letter)) if letter.is_ascii_alphabetic() && is_word_start(third) => {
            PathRoot::Drive
        }
        // `/c` from Git Bash or `/mnt/c` from WSL: a one-letter folder.
        (Some(letter), Some(separator))
            if letter.is_ascii_alphabetic() && SEPARATORS.contains(&separator) =>
        {
            PathRoot::Drive
        }
        (Some(character), _) if is_segment_char(character) => PathRoot::Relative,
        _ => PathRoot::Top,
    }
}

/// The user name of every home folder in `text` that is not a shared or scrubbed one. A home
/// folder is a `users` or `home` folder right after a drive (any case), or `/Users` or `/home` at
/// the top of an absolute path (as macOS and Linux spell them). A `users` folder inside a
/// relative path or a web address is not a home folder. Separators may repeat or mix, as in
/// escaped JSON.
fn home_users_in(text: &str, lowered: &str) -> Vec<String> {
    let mut users = Vec::new();
    for (word, top_spelling) in [("users", "Users"), ("home", "home")] {
        for (index, _) in lowered.match_indices(word) {
            let head = lowered.get(..index).unwrap_or_default();
            let tail = lowered.get(index + word.len()..).unwrap_or_default();
            if !head.ends_with(SEPARATORS) || !tail.starts_with(SEPARATORS) {
                continue;
            }
            let is_home = match path_root(head.trim_end_matches(SEPARATORS)) {
                PathRoot::Drive => true,
                PathRoot::Top => text.get(index..index + word.len()) == Some(top_spelling),
                PathRoot::Relative => false,
            };
            let user: String = tail
                .trim_start_matches(SEPARATORS)
                .chars()
                .take_while(|character| is_name_char(*character))
                .collect();
            if is_home && !user.is_empty() && !SHARED_HOME_NAMES.contains(&user.as_str()) {
                users.push(user);
            }
        }
    }
    users
}

/// Every hyphen-encoded home folder in `lowered` text, as the marker without its first hyphen and
/// the name characters after it. The marker must start a folder name (see
/// [`ENCODED_FOLDER_STARTS`]), or, for `-users-`, follow a drive letter and a hyphen (`c--users-`).
fn encoded_homes_in(lowered: &str) -> Vec<String> {
    let mut folders = Vec::new();
    for marker in ENCODED_MARKERS {
        for (index, _) in lowered.match_indices(marker) {
            let head = lowered.get(..index).unwrap_or_default();
            let mut back = head.chars().rev();
            let (last, second, third) = (back.next(), back.next(), back.next());
            let is_drive_form = *marker == "-users-"
                && last == Some('-')
                && second.is_some_and(|letter| letter.is_ascii_alphabetic())
                && third.is_none_or(|before| !before.is_alphanumeric());
            let starts_folder = last.is_none_or(|before| ENCODED_FOLDER_STARTS.contains(&before));
            if !is_drive_form && !starts_folder {
                continue;
            }
            let rest = lowered.get(index + marker.len()..).unwrap_or_default();
            let name: String = rest
                .chars()
                .take_while(|character| is_name_char(*character))
                .collect();
            folders.push(format!("{}{name}", marker.trim_start_matches('-')));
        }
    }
    folders
}

/// Every default Windows machine name in `text`: a machine prefix at the start of a word, then
/// exactly the expected number of letters and digits, holding a digit or written in capitals as
/// Windows writes it. A word such as `desktop-classic` is left out.
fn machine_names_in(text: &str, lowered: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (prefix, suffix_len) in MACHINE_NAMES {
        for (index, _) in lowered.match_indices(prefix) {
            let before = lowered
                .get(..index)
                .and_then(|head| head.chars().next_back());
            if before.is_some_and(char::is_alphanumeric) {
                continue;
            }
            let rest = text.get(index + prefix.len()..).unwrap_or_default();
            let suffix: String = rest
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            let is_machine_like = suffix.chars().any(|character| character.is_ascii_digit())
                || suffix
                    .chars()
                    .all(|character| character.is_ascii_uppercase());
            if suffix.len() == *suffix_len && is_machine_like {
                names.push(format!("{prefix}{}", suffix.to_ascii_lowercase()));
            }
        }
    }
    names
}

/// Every email address in `text`: a local part, `@`, and a domain whose first label holds a
/// letter and whose last label is two or more letters. The `git` user of an SSH remote
/// (`git@github.com:owner/repo`) and a version after a package name (`pkg@1.2.3-beta.rc`) are not
/// addresses.
fn emails_in(text: &str) -> Vec<String> {
    let is_local =
        |character: char| character.is_ascii_alphanumeric() || "._%+-".contains(character);
    let is_domain = |character: char| character.is_ascii_alphanumeric() || ".-".contains(character);
    let mut emails = Vec::new();
    for (index, _) in text.match_indices('@') {
        let before = text.get(..index).unwrap_or_default();
        let after = text.get(index + 1..).unwrap_or_default();
        let local: String = before
            .chars()
            .rev()
            .take_while(|character| is_local(*character))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let domain: String = after
            .chars()
            .take_while(|character| is_domain(*character))
            .collect();
        let domain = domain.trim_end_matches('.');
        let mut labels = domain.split('.');
        let first_has_letter = labels.next().is_some_and(|label| {
            label
                .chars()
                .any(|character| character.is_ascii_alphabetic())
        });
        let suffix = domain.rsplit('.').next().unwrap_or("");
        let has_suffix = domain.contains('.')
            && suffix.len() >= 2
            && suffix
                .chars()
                .all(|character| character.is_ascii_alphabetic());
        if !local.is_empty() && local != "git" && first_has_letter && has_suffix {
            emails.push(format!("{local}@{domain}"));
        }
    }
    emails
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed ids, the scrubbed user's and shared homes, relative paths, web addresses, SSH remotes,
    /// package versions and plain words raise nothing.
    #[test]
    fn scrubbed_text_is_clean() {
        let clean = [
            "C:\\Users\\dev\\.codex\\rollout-00000000-0000-4000-8000-000000000001.jsonl",
            "/home/dev/project and /c/Users/dev",
            "C:\\Users\\dev\\.claude\\projects\\C--repo\\x.jsonl",
            "toolu_fixture0001 call_fixture0002 exec-00000000-0000-4000-8000-000000000003",
            "short-0000-4000, 2026-10-29T06-03-56, call_sites, decorators like @property",
            "the home folder is `C:\\Users\\dev`; the rest is prose",
            "take-home-assignment.md, a work-from-home policy, the --home-dir flag",
            "pass the -home-dir flag",
            "see https://api.github.com/users/octocat and (http://example.com/home/page)",
            "GET /users/list returns users",
            "src/routes/users/profile.ts and app/home/page.tsx and ~/projects/home/x",
            "C:/Users/Public/Desktop and /Users/Shared/x",
            "git@github.com:org/repo.git and pkg@1.2.3-beta.rc",
            "a desktop-classic theme, msg_count, req_body, gAAAAA_fixture_encrypted_message",
        ];
        for text in clean {
            assert_eq!(text_problems(text), Vec::<String>::new(), "{text}");
        }
    }

    /// Live ids and tokens, other users' homes in any spelling and separator mix, encoded home
    /// folders, machine names and email addresses are each reported once.
    #[test]
    fn unscrubbed_text_is_reported() {
        let dirty = [
            "rollout-01a18642-06c8-7022-ac9c-fca4853cfe2c.jsonl",
            "exec-5aa55f07-61d9-4d37-baed-65ba37f4185f",
            "toolu_01AbCdEfGhIjKlMn",
            "call_76e195851d464a06bb54da631b1d40da",
            "msg_01XyZaBcDeFgHiJkLmNoPq",
            "req_011CUZkYzAbCdEfGhIjKl",
            "gAAAAABpQwErTyUiOpAsDfGhJkL",
            "/home/alice/project",
            "/Users/alice/proj",
            "C:\\Users\\devon\\notes",
            "c:\\users\\alice\\notes",
            "C:\\\\Users\\\\alice\\\\proj",
            "C:/Users\\alice/x",
            "C:\\Users/alice/x",
            "{\"p\":\"\\/home\\/alice\\/x\"}",
            "/c/Users/alice/x",
            "/mnt/c/Users/alice/x",
            "https://example.com,/home/alice/x",
            "projects\\C--Users-alice-Desktop-repo\\x.jsonl",
            "projects\\C--Users-Dev-Smith-Desktop-x\\y.jsonl",
            "projects/-home-alice-repo/x.jsonl",
            "dir=-Users-alice-proj",
            "whoami printed desktop-7f3kq2m",
            "host DESKTOP-ABCDEFG",
            "host WIN-ABCDEFGHIJK",
            "write to someone.else@example.org today",
        ];
        for text in dirty {
            assert_eq!(
                text_problems(text).len(),
                1,
                "{text}: {:?}",
                text_problems(text)
            );
        }
    }

    /// Keys are scanned as well as values, ids and hashes are checked wherever their key appears,
    /// and the folder fields must be scrubbed.
    #[test]
    fn payload_keys_ids_and_folders_are_checked() {
        let clean = serde_json::json!({
            "session_id": "00000000-0000-4000-8000-000000000001",
            "cwd": "C:\\repo",
            "transcript_path": "C:\\Users\\dev\\.claude\\projects\\C--repo\\x.jsonl",
            "agent_transcript_path": "C:\\Users\\dev\\.claude\\projects\\C--repo\\y.jsonl",
            "tool_response": {"agentId": "fixtureagent0001", "harnessSectionHash": "0000"}
        });
        assert_eq!(payload_problems(&clean), Vec::<String>::new());
        let resumed = serde_json::json!({
            "cwd": "C:\\repo\\sub",
            "transcript_path": "\\\\?\\C:\\Users\\dev\\.codex\\x.jsonl"
        });
        assert_eq!(payload_problems(&resumed), Vec::<String>::new());
        let dirty = [
            serde_json::json!({"cwd": "C:\\work\\repo"}),
            serde_json::json!({"cwd": "/repository-of-alice"}),
            serde_json::json!({"transcript_path": "D:\\logs\\x.jsonl"}),
            serde_json::json!({"agent_transcript_path": "D:\\agents\\x.jsonl"}),
            serde_json::json!({"nested": {"agent_id": "a1b2c3d4e5f6a7b8c"}}),
            serde_json::json!({"turn_id": 7}),
            serde_json::json!({"files": {"/home/alice/x.rs": 1}}),
            serde_json::json!({"tool_response": {"harnessSectionHash": "9f3a"}}),
            serde_json::json!({"tool_response": "{\"path\":\"C:\\\\Users\\\\alice\\\\x\"}"}),
        ];
        for payload in dirty {
            assert_eq!(payload_problems(&payload).len(), 1, "{payload}");
        }
    }

    /// The three path roots, worked out by hand.
    #[test]
    fn path_roots_are_told_apart() {
        let cases = [
            ("c:", PathRoot::Drive),
            ("\\\\?\\c:", PathRoot::Drive),
            ("/c", PathRoot::Drive),
            ("/mnt/c", PathRoot::Drive),
            ("", PathRoot::Top),
            ("get ", PathRoot::Top),
            ("{\"p\":\"", PathRoot::Top),
            ("https://example.com,", PathRoot::Top),
            ("src/routes", PathRoot::Relative),
            ("https://api.github.com", PathRoot::Relative),
            ("ac:", PathRoot::Top),
        ];
        for (head, expected) in cases {
            assert_eq!(path_root(head), expected, "{head:?}");
        }
    }
}
