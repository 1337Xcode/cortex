//! What a scrubbed fixture may contain.
//!
//! A captured hook payload carries the capturing machine's paths, user name, machine name and live
//! ids. The scrubber replaces them with fixed values before a fixture is committed; these checks
//! find anything it missed, in every key and value of a payload and in the text of a fixture
//! README.

use std::ops::Range;

/// Folders a scrubbed `cwd` may be, or be inside.
pub const SCRUBBED_REPO_ROOTS: &[&str] = &["C:\\repo", "/repo"];

/// Folder prefixes a scrubbed transcript path may start with, including the Windows verbatim form
/// (`\\?\`) Codex writes when it resumes a session.
pub const SCRUBBED_HOMES: &[&str] = &["C:\\Users\\dev\\", "\\\\?\\C:\\Users\\dev\\", "/home/dev/"];

/// The only user name a scrubbed home folder may show.
pub const SCRUBBED_USER: &str = "dev";

/// Prefix of every scrubbed UUID-style id.
pub const SCRUBBED_UUID_PREFIX: &str = "00000000-0000-4000-8000-";

/// Lower-case text that a user name follows in a plain path, with the separators that end the
/// name. Separators repeated after the marker are skipped, so a path whose backslashes were
/// escaped once more (`C:\\Users\\name`, as in JSON held inside a string) is still read.
const PATH_MARKERS: &[(&str, &[char])] = &[
    ("\\users\\", &['\\', '/']),
    ("/users/", &['/', '\\']),
    ("/home/", &['/', '\\']),
];

/// Lower-case markers of the hyphen-encoded folder names Claude Code gives transcript folders
/// (`C--Users-<name>-...` on Windows, `-Users-<name>-...` on macOS, `-home-<name>-...` on Linux).
/// The scrubber collapses a whole encoded folder to `C--repo`, and the name cannot be told apart
/// from the path after it, so every encoded home folder is reported.
const ENCODED_MARKERS: &[&str] = &["-users-", "-home-"];

/// Web address schemes; a `/users/` or `/home/` inside a web address is a web path, not a folder.
const WEB_SCHEMES: &[&str] = &["http://", "https://"];

/// Lower-case prefixes of the names Windows gives a new machine: the prefix, then
/// [`MACHINE_SUFFIX_LEN`] letters and digits, such as `DESKTOP-7F3KQ2M`.
const MACHINE_PREFIXES: &[&str] = &["desktop-", "laptop-"];

/// Length of the random part of a default Windows machine name.
const MACHINE_SUFFIX_LEN: usize = 7;

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
/// replaces every digit with `0`.
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
    if let Some(transcript) = field("transcript_path")
        && !SCRUBBED_HOMES
            .iter()
            .any(|home| transcript.starts_with(home))
    {
        problems.push(format!(
            "transcript path {transcript:?} is not under a scrubbed home"
        ));
    }
    let mut pairs = Vec::new();
    key_values(payload, &mut pairs);
    for (key, value) in &pairs {
        if ID_KEYS.contains(key)
            && !value.is_empty()
            && !FIXED_ID_PREFIXES
                .iter()
                .any(|prefix| value.starts_with(prefix))
        {
            problems.push(format!("{key} {value:?} is not a fixed id"));
        }
        if ZEROED_KEYS.contains(key) && !value.chars().all(|character| character == '0') {
            problems.push(format!("{key} {value:?} is not zeroed"));
        }
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
        home_users_in(&lowered)
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

/// Whether `path` is `root` or a path inside it.
fn is_inside(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['\\', '/']))
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

/// Every key whose value is a string, with that value, at any depth.
fn key_values<'a>(value: &'a serde_json::Value, out: &mut Vec<(&'a str, &'a str)>) {
    match value {
        serde_json::Value::Array(items) => items.iter().for_each(|item| key_values(item, out)),
        serde_json::Value::Object(fields) => {
            for (key, field) in fields {
                if let Some(text) = field.as_str() {
                    out.push((key.as_str(), text));
                }
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

/// The character just before byte offset `index` of `text`, if any.
fn char_before(text: &str, index: usize) -> Option<char> {
    text.get(..index).and_then(|head| head.chars().next_back())
}

/// The byte ranges of the web addresses in `lowered`: from the scheme to the next space, quote,
/// angle bracket or parenthesis.
fn web_address_spans(lowered: &str) -> Vec<Range<usize>> {
    let ends_address =
        |character: char| character.is_whitespace() || "\"'`<>()".contains(character);
    let mut spans = Vec::new();
    for scheme in WEB_SCHEMES {
        for (start, _) in lowered.match_indices(scheme) {
            let end = lowered
                .get(start..)
                .and_then(|rest| rest.find(ends_address))
                .map_or(lowered.len(), |offset| start + offset);
            spans.push(start..end);
        }
    }
    spans
}

/// The user name after every plain home-folder marker in `lowered` text, when it is not the
/// scrubbed user. A name ends at its marker's separator or at the first character a user name
/// cannot hold, such as a closing quote. Markers inside web addresses are skipped.
fn home_users_in(lowered: &str) -> Vec<String> {
    let web = web_address_spans(lowered);
    let mut users = Vec::new();
    for (marker, separators) in PATH_MARKERS {
        for (index, _) in lowered.match_indices(marker) {
            if web.iter().any(|span| span.contains(&index)) {
                continue;
            }
            let rest = lowered.get(index + marker.len()..).unwrap_or_default();
            let user: String = rest
                .trim_start_matches(*separators)
                .chars()
                .take_while(|character| !separators.contains(character) && is_name_char(*character))
                .collect();
            if !user.is_empty() && user != SCRUBBED_USER {
                users.push(user);
            }
        }
    }
    users
}

/// Every hyphen-encoded home folder in `lowered` text, as the marker without its first hyphen and
/// the name characters after it. The marker must start a folder name: after a path separator, a
/// quote, a space or the start of the text, or, for `-users-`, after a drive letter and a hyphen
/// (`c--users-`). This leaves out words such as `take-home-pay` and flags such as `--home-dir`.
fn encoded_homes_in(lowered: &str) -> Vec<String> {
    let starts_name = |character: char| "/\\\"'`".contains(character) || character.is_whitespace();
    let mut folders = Vec::new();
    for marker in ENCODED_MARKERS {
        for (index, _) in lowered.match_indices(marker) {
            let head = lowered.get(..index).unwrap_or_default();
            let is_drive_form = *marker == "-users-" && {
                let mut back = head.chars().rev();
                matches!(
                    (back.next(), back.next(), back.next()),
                    (Some('-'), Some(letter), before)
                        if letter.is_ascii_alphabetic()
                            && before.is_none_or(|character| !character.is_alphanumeric())
                )
            };
            if !is_drive_form && !char_before(lowered, index).is_none_or(starts_name) {
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
/// exactly [`MACHINE_SUFFIX_LEN`] letters and digits holding a digit or written in capitals, as
/// Windows writes it. A word such as `desktop-classic` is left out.
fn machine_names_in(text: &str, lowered: &str) -> Vec<String> {
    let mut names = Vec::new();
    for prefix in MACHINE_PREFIXES {
        for (index, _) in lowered.match_indices(prefix) {
            if char_before(lowered, index).is_some_and(char::is_alphanumeric) {
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
            if suffix.len() == MACHINE_SUFFIX_LEN && is_machine_like {
                names.push(format!("{prefix}{}", suffix.to_ascii_lowercase()));
            }
        }
    }
    names
}

/// Every email address in `text`: a local part, `@`, and a domain with a dot and a letter suffix.
/// The `git` user of an SSH remote (`git@github.com:owner/repo`) is not an address.
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
        let suffix = domain
            .trim_end_matches('.')
            .rsplit('.')
            .next()
            .unwrap_or("");
        let has_suffix = domain.contains('.')
            && suffix.len() >= 2
            && suffix
                .chars()
                .all(|character| character.is_ascii_alphabetic());
        if !local.is_empty() && local != "git" && has_suffix {
            emails.push(format!("{local}@{}", domain.trim_end_matches('.')));
        }
    }
    emails
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed ids, the scrubbed user's paths, web addresses, SSH remotes and plain words raise
    /// nothing.
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
            "see https://api.github.com/users/octocat and (http://example.com/home/page)",
            "git@github.com:org/repo.git",
            "a desktop-classic theme, msg_count, req_body, gAAAAA_fixture_encrypted_message",
        ];
        for text in clean {
            assert_eq!(text_problems(text), Vec::<String>::new(), "{text}");
        }
    }

    /// Live ids and tokens, other users' folders in any spelling, encoded home folders, machine
    /// names and email addresses are each reported once.
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
            "C:\\Users\\devon\\notes",
            "c:\\users\\alice\\notes",
            "C:\\\\Users\\\\alice\\\\proj",
            "projects\\C--Users-alice-Desktop-repo\\x.jsonl",
            "projects\\C--Users-Dev-Smith-Desktop-x\\y.jsonl",
            "projects/-home-alice-repo/x.jsonl",
            "whoami printed desktop-7f3kq2m",
            "host DESKTOP-ABCDEFG",
            "write to someone.else@example.org today",
        ];
        for text in dirty {
            assert_eq!(text_problems(text).len(), 1, "{text}");
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
            serde_json::json!({"nested": {"agent_id": "a1b2c3d4e5f6a7b8c"}}),
            serde_json::json!({"files": {"/home/alice/x.rs": 1}}),
            serde_json::json!({"tool_response": {"harnessSectionHash": "9f3a"}}),
            serde_json::json!({"tool_response": "{\"path\":\"C:\\\\Users\\\\alice\\\\x\"}"}),
        ];
        for payload in dirty {
            assert_eq!(payload_problems(&payload).len(), 1, "{payload}");
        }
    }
}
