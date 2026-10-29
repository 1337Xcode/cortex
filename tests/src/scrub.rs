//! What a scrubbed fixture may contain.
//!
//! A captured hook payload carries the capturing machine's paths, user name and live ids. The
//! scrubber replaces them with fixed values before a fixture is committed; these checks find
//! anything it missed, in every key and value of a payload and in the text of a fixture README.

/// Folder prefixes a scrubbed `cwd` may start with.
pub const SCRUBBED_REPO_ROOTS: &[&str] = &["C:\\repo", "/repo"];

/// Folder prefixes a scrubbed transcript path may start with, including the Windows verbatim form
/// (`\\?\`) Codex writes when it resumes a session.
pub const SCRUBBED_HOMES: &[&str] = &["C:\\Users\\dev\\", "\\\\?\\C:\\Users\\dev\\", "/home/dev/"];

/// The only user name a scrubbed home folder may show.
pub const SCRUBBED_USER: &str = "dev";

/// Prefix of every scrubbed UUID-style id.
pub const SCRUBBED_UUID_PREFIX: &str = "00000000-0000-4000-8000-";

/// Lower-case text that a user name follows, with the characters that may end the name: plain
/// paths on Windows and Unix, and the hyphen-encoded folder names Claude Code gives transcript
/// folders (`C--Users-<name>-...`, `-home-<name>-...`).
const HOME_MARKERS: &[(&str, &[char])] = &[
    ("\\users\\", &['\\', '/']),
    ("/users/", &['/', '\\']),
    ("/home/", &['/']),
    ("-users-", &['-']),
    ("-home-", &['-']),
];

/// Keys whose values are session, prompt, turn, subagent or tool-call ids.
const ID_KEYS: &[&str] = &[
    "session_id",
    "prompt_id",
    "turn_id",
    "agent_id",
    "agentId",
    "tool_use_id",
];

/// The forms a scrubbed id takes.
const FIXED_ID_PREFIXES: &[&str] = &[
    SCRUBBED_UUID_PREFIX,
    "exec-00000000-0000-4000-8000-",
    "toolu_fixture",
    "call_fixture",
    "fixtureagent",
];

/// Ids that also appear inside text: the prefix, the fixed form, and the shortest run of letters
/// and digits after the prefix that counts as an id. Codex call ids are 32 hex digits; a shorter
/// run after `call_` is an ordinary word such as `call_sites`.
const EMBEDDED_IDS: &[(&str, &str, usize)] = &[
    ("toolu_", "toolu_fixture", 1),
    ("call_", "call_fixture", 16),
];

/// Length of a UUID in its hyphenated text form: `8-4-4-4-12` hexadecimal digits.
const UUID_LEN: usize = 36;

/// Every problem with one parsed payload: its folder and session fields, every id-valued key, and
/// the text of every key and string value.
#[must_use]
pub fn payload_problems(payload: &serde_json::Value) -> Vec<String> {
    let mut problems = Vec::new();
    let field = |key: &str| payload.get(key).and_then(serde_json::Value::as_str);
    if let Some(cwd) = field("cwd")
        && !SCRUBBED_REPO_ROOTS.iter().any(|root| cwd.starts_with(root))
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
    }
    let mut texts = Vec::new();
    strings_in(payload, &mut texts);
    problems.extend(texts.into_iter().flat_map(text_problems));
    problems
}

/// Every unscrubbed id, home folder or email address in one piece of text.
#[must_use]
pub fn text_problems(text: &str) -> Vec<String> {
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
        home_users_in(text)
            .into_iter()
            .map(|user| format!("home folder of user {user:?} is not scrubbed")),
    );
    problems.extend(
        emails_in(text)
            .into_iter()
            .map(|email| format!("email address {email:?}")),
    );
    problems
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

/// Every embedded tool or call id that is not in its fixed form.
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

/// The user name after every home-folder marker, when it is not the scrubbed user. Matching
/// ignores case, since the same path can appear in any case on Windows. A name ends at its
/// marker's separator or at the first character a user name cannot hold, such as a closing quote.
fn home_users_in(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut users = Vec::new();
    for (marker, separators) in HOME_MARKERS {
        for (index, _) in lowered.match_indices(marker) {
            let rest = lowered.get(index + marker.len()..).unwrap_or_default();
            let user: String = rest
                .chars()
                .take_while(|character| {
                    !separators.contains(character)
                        && (character.is_alphanumeric() || "._-".contains(*character))
                })
                .collect();
            if !user.is_empty() && user != SCRUBBED_USER {
                users.push(user);
            }
        }
    }
    users
}

/// Every email address in `text`: a local part, `@`, and a domain with a dot and a letter suffix.
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
        if !local.is_empty() && has_suffix {
            emails.push(format!("{local}@{}", domain.trim_end_matches('.')));
        }
    }
    emails
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed ids, the scrubbed user's paths and plain words raise nothing.
    #[test]
    fn scrubbed_text_is_clean() {
        let clean = [
            "C:\\Users\\dev\\.codex\\rollout-00000000-0000-4000-8000-000000000001.jsonl",
            "/home/dev/project and /c/Users/dev",
            "C:\\Users\\dev\\.claude\\projects\\C--repo\\x.jsonl",
            "toolu_fixture0001 call_fixture0002 exec-00000000-0000-4000-8000-000000000003",
            "short-0000-4000, 2026-10-29T06-03-56, call_sites, decorators like @property",
            "the home folder is `C:\\Users\\dev`; the rest is prose",
        ];
        for text in clean {
            assert_eq!(text_problems(text), Vec::<String>::new(), "{text}");
        }
    }

    /// Live ids, other users' folders in any spelling, and email addresses are each reported once.
    #[test]
    fn unscrubbed_text_is_reported() {
        let dirty = [
            "rollout-01a18642-06c8-7022-ac9c-fca4853cfe2c.jsonl",
            "exec-5aa55f07-61d9-4d37-baed-65ba37f4185f",
            "toolu_01AbCdEfGhIjKlMn",
            "call_76e195851d464a06bb54da631b1d40da",
            "/home/alice/project",
            "C:\\Users\\devon\\notes",
            "c:\\users\\alice\\notes",
            "projects\\C--Users-alice-Desktop-repo\\x.jsonl",
            "projects/-home-alice-repo/x.jsonl",
            "write to someone.else@example.org today",
        ];
        for text in dirty {
            assert_eq!(text_problems(text).len(), 1, "{text}");
        }
    }

    /// Keys are scanned as well as values, ids are checked wherever their key appears, and the
    /// folder fields must be scrubbed.
    #[test]
    fn payload_keys_ids_and_folders_are_checked() {
        let clean = serde_json::json!({
            "session_id": "00000000-0000-4000-8000-000000000001",
            "cwd": "C:\\repo",
            "transcript_path": "C:\\Users\\dev\\.claude\\projects\\C--repo\\x.jsonl",
            "tool_response": {"agentId": "fixtureagent0001", "agent_type": ""}
        });
        assert_eq!(payload_problems(&clean), Vec::<String>::new());
        let resumed = serde_json::json!({
            "transcript_path": "\\\\?\\C:\\Users\\dev\\.codex\\x.jsonl"
        });
        assert_eq!(payload_problems(&resumed), Vec::<String>::new());
        let dirty = [
            serde_json::json!({"cwd": "C:\\work\\repo"}),
            serde_json::json!({"transcript_path": "D:\\logs\\x.jsonl"}),
            serde_json::json!({"nested": {"agent_id": "a1b2c3d4e5f6a7b8c"}}),
            serde_json::json!({"files": {"/home/alice/x.rs": 1}}),
        ];
        for payload in dirty {
            assert_eq!(payload_problems(&payload).len(), 1, "{payload}");
        }
    }
}
