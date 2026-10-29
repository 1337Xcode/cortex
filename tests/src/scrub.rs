//! What a scrubbed fixture may contain.
//!
//! A captured hook payload carries the capturing machine's paths, user name, machine name, live
//! ids and sometimes a secret from a tool's output. The scrubber replaces them with fixed values
//! before a fixture is committed; these checks find anything it missed, in every key and value of
//! a payload and in the text of a fixture README. How home folders are recognised is described in
//! [`crate::homes`].

use crate::homes;

/// Folders a scrubbed `cwd` or `workdir` may be, or be inside.
pub const SCRUBBED_REPO_ROOTS: &[&str] = &["C:\\repo", "/repo"];

/// Folder prefixes a scrubbed transcript path may start with, including the Windows verbatim form
/// (`\\?\`) Codex writes when it resumes a session.
pub const SCRUBBED_HOMES: &[&str] = &["C:\\Users\\dev\\", "\\\\?\\C:\\Users\\dev\\", "/home/dev/"];

/// The only user name a scrubbed home folder may show.
pub const SCRUBBED_USER: &str = "dev";

/// Prefix of every scrubbed UUID-style id; twelve digits follow it.
pub const SCRUBBED_UUID_PREFIX: &str = "00000000-0000-4000-8000-";

/// Keys whose values are working folders, which must sit inside a scrubbed repository root.
const FOLDER_KEYS: &[&str] = &["cwd", "workdir"];

/// Keys whose values are transcript paths, which must sit under a scrubbed home.
const TRANSCRIPT_KEYS: &[&str] = &["transcript_path", "agent_transcript_path"];

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

/// The fixed ids the scrubber writes: a prefix, then exactly this many digits.
const FIXED_IDS: &[(&str, usize)] = &[
    (SCRUBBED_UUID_PREFIX, 12),
    ("exec-00000000-0000-4000-8000-", 12),
    ("toolu_fixture", 4),
    ("call_fixture", 4),
    ("fixtureagent", 4),
    ("msg_fixture", 4),
    ("req_fixture", 4),
];

/// Ids and tokens that also appear inside text: the prefix, and the shortest run of letters and
/// digits after it that counts as an id. Codex call ids are 32 hex digits; a shorter run after
/// `call_` is an ordinary word such as `call_sites`. Anthropic message and request ids start
/// `msg_` and `req_`; Codex passes a subagent's context as an encrypted Fernet token, which
/// starts `gAAAAA` (the scrubber's placeholder has no letters or digits right after it).
const EMBEDDED_IDS: &[(&str, usize)] = &[
    ("toolu_", 1),
    ("call_", 16),
    ("msg_", 16),
    ("req_", 16),
    ("gAAAAA", 16),
];

/// Prefixes of credentials that tool output can print, and the shortest run of letters, digits,
/// `_` and `-` after the prefix that counts as one: Anthropic and OpenAI keys, GitHub tokens,
/// Slack tokens, AWS access key ids and Google API keys.
const SECRET_TOKENS: &[(&str, usize)] = &[
    ("sk-ant-", 20),
    ("sk-proj-", 20),
    ("ghp_", 30),
    ("gho_", 30),
    ("ghs_", 30),
    ("ghu_", 30),
    ("github_pat_", 30),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("AKIA", 16),
    ("AIza", 30),
];

/// The names Windows gives a new machine, as a lower-case prefix and the length of the random
/// part after it: `DESKTOP-7F3KQ2M`, `LAPTOP-7F3KQ2M`, and `WIN-` with eleven on servers.
const MACHINE_NAMES: &[(&str, usize)] = &[("desktop-", 7), ("laptop-", 7), ("win-", 11)];

/// Length of a UUID in its hyphenated text form: `8-4-4-4-12` hexadecimal digits.
const UUID_LEN: usize = 36;

/// Every problem with one parsed payload: its folder and transcript fields, every id-valued and
/// hash-valued key, and the text of every key and string value, at any depth.
#[must_use]
pub fn payload_problems(payload: &serde_json::Value) -> Vec<String> {
    let mut pairs = Vec::new();
    key_values(payload, &mut pairs);
    let mut problems: Vec<String> = pairs
        .iter()
        .flat_map(|(key, value)| value_problems(key, value))
        .collect();
    let mut texts = Vec::new();
    strings_in(payload, &mut texts);
    problems.extend(texts.into_iter().flat_map(text_problems));
    problems
}

/// Every unscrubbed id, credential, home folder, machine name or email address in one piece of
/// text.
#[must_use]
pub fn text_problems(text: &str) -> Vec<String> {
    let mut problems: Vec<String> = uuids_in(text)
        .into_iter()
        .filter(|id| !is_fixed_id(id))
        .map(|id| format!("id {id} is not a fixed id"))
        .collect();
    problems.extend(
        embedded_ids_in(text)
            .into_iter()
            .map(|id| format!("id {id} is not a fixed id")),
    );
    problems.extend(
        secret_tokens_in(text)
            .into_iter()
            .map(|prefix| format!("a credential starting {prefix:?}")),
    );
    problems.extend(
        homes::home_users_in(text)
            .into_iter()
            .map(|user| format!("home folder of user {user:?} is not scrubbed")),
    );
    problems.extend(
        homes::encoded_homes_in(text)
            .into_iter()
            .map(|folder| format!("encoded home folder {folder:?} is not scrubbed")),
    );
    problems.extend(
        machine_names_in(text)
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

/// The problems with one key and its value: a folder or transcript path outside the scrubbed
/// ones, an id that is not in a fixed form or not a string, or an opaque hash that is not zeroed.
fn value_problems(key: &str, value: &serde_json::Value) -> Vec<String> {
    let mut problems = Vec::new();
    let text = value.as_str();
    if FOLDER_KEYS.contains(&key)
        && let Some(folder) = text
        && !SCRUBBED_REPO_ROOTS
            .iter()
            .any(|root| is_inside(folder, root))
    {
        problems.push(format!(
            "{key} {folder:?} is not a scrubbed repository path"
        ));
    }
    if TRANSCRIPT_KEYS.contains(&key)
        && let Some(path) = text
        && !SCRUBBED_HOMES.iter().any(|home| path.starts_with(home))
    {
        problems.push(format!("{key} {path:?} is not under a scrubbed home"));
    }
    if ID_KEYS.contains(&key) {
        match value {
            serde_json::Value::Null => {}
            serde_json::Value::String(id) => {
                if !id.is_empty() && !is_fixed_id(id) {
                    problems.push(format!("{key} {id:?} is not a fixed id"));
                }
            }
            other => problems.push(format!("{key} {other} is not a string id")),
        }
    }
    if ZEROED_KEYS.contains(&key)
        && !text.is_some_and(|hash| hash.chars().all(|character| character == '0'))
    {
        problems.push(format!("{key} {value} is not zeroed"));
    }
    problems
}

/// Whether `id` is exactly one of the fixed ids the scrubber writes.
fn is_fixed_id(id: &str) -> bool {
    FIXED_IDS.iter().any(|(prefix, digits)| {
        id.strip_prefix(prefix).is_some_and(|rest| {
            rest.len() == *digits && rest.chars().all(|character| character.is_ascii_digit())
        })
    })
}

/// Whether `path` is `root` or a path inside it.
fn is_inside(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '\\']))
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

/// The run of characters accepted by `is_part` right after byte offset `index` of `text`.
fn run_after(text: &str, index: usize, is_part: impl Fn(char) -> bool) -> String {
    text.get(index..)
        .unwrap_or_default()
        .chars()
        .take_while(|character| is_part(*character))
        .collect()
}

/// Every embedded id or token that is not in its fixed form.
fn embedded_ids_in(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for (prefix, min_len) in EMBEDDED_IDS {
        for (index, _) in text.match_indices(prefix) {
            let run = run_after(text, index + prefix.len(), |character| {
                character.is_ascii_alphanumeric()
            });
            let id = format!("{prefix}{run}");
            if run.len() >= *min_len && !is_fixed_id(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// The prefix of every credential-shaped token in `text`.
fn secret_tokens_in(text: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    for (prefix, min_len) in SECRET_TOKENS {
        for (index, _) in text.match_indices(prefix) {
            let run = run_after(text, index + prefix.len(), |character| {
                character.is_ascii_alphanumeric() || "_-".contains(character)
            });
            if run.len() >= *min_len {
                found.push(*prefix);
            }
        }
    }
    found
}

/// Every default Windows machine name in `text`: a machine prefix at the start of a word, then
/// exactly the expected number of letters and digits, holding a digit or written in capitals as
/// Windows writes it. A word such as `desktop-classic` is left out.
fn machine_names_in(text: &str) -> Vec<String> {
    // ASCII lowering keeps every byte offset, so positions found in `lowered` hold in `text`.
    let lowered = text.to_ascii_lowercase();
    let mut names = Vec::new();
    for (prefix, suffix_len) in MACHINE_NAMES {
        for (index, _) in lowered.match_indices(prefix) {
            let before = lowered
                .get(..index)
                .and_then(|head| head.chars().next_back());
            if before.is_some_and(char::is_alphanumeric) {
                continue;
            }
            let suffix = run_after(text, index + prefix.len(), |character| {
                character.is_ascii_alphanumeric()
            });
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
        let local: String = before
            .chars()
            .rev()
            .take_while(|character| is_local(*character))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let domain = run_after(text, index + 1, is_domain);
        let domain = domain.trim_end_matches('.');
        let first_has_letter = domain.split('.').next().is_some_and(|label| {
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

    /// Fixed ids, the scrubbed user's and shared homes, relative and project paths, web
    /// addresses, SSH remotes, package versions and plain words raise nothing.
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
            "the users/list route, src/routes/users/profile.ts and app/home/page.tsx",
            "C:/Users/Public/Desktop and /Users/Shared/x and /repo/src/users/x.ts",
            "git@github.com:org/repo.git and pkg@1.2.3-beta.rc",
            "a desktop-classic theme, msg_count, req_body, gAAAAA_fixture_encrypted_message",
            "sk-short and ghp_short and the AKIA prefix",
        ];
        for text in clean {
            assert_eq!(text_problems(text), Vec::<String>::new(), "{text}");
        }
    }

    /// Live ids and tokens, credentials, other users' homes, encoded home folders, machine names
    /// and email addresses are each reported once.
    #[test]
    fn unscrubbed_text_is_reported() {
        let dirty = [
            "rollout-01a18642-06c8-7022-ac9c-fca4853cfe2c.jsonl",
            "rollout-00000000-0000-4000-8000-00000000000a.jsonl",
            "exec-5aa55f07-61d9-4d37-baed-65ba37f4185f",
            "toolu_01AbCdEfGhIjKlMn",
            "call_76e195851d464a06bb54da631b1d40da",
            "msg_01XyZaBcDeFgHiJkLmNoPq",
            "req_011CUZkYzAbCdEfGhIjKl",
            "gAAAAABpQwErTyUiOpAsDfGhJkL",
            "/home/alice/project",
            "/mnt/c/Users/alice/x",
            "\\\\wsl$\\Ubuntu\\home\\alice\\repo",
            "C:\\\\Users\\\\alice\\\\proj",
            "projects\\C--Users-alice-Desktop-repo\\x.jsonl",
            "C:\\Users\\dev\\.claude\\projects\\-mnt-c-Users-alice-repo\\x.jsonl",
            "whoami printed desktop-7f3kq2m",
            "host DESKTOP-ABCDEFG",
            "host WIN-ABCDEFGHIJK",
            "write to someone.else@example.org today",
        ];
        // Credential-shaped values are built at run time, so this file holds none.
        let anthropic = format!("key sk-ant-{}", "a".repeat(24));
        let github = format!("token ghp_{}", "B".repeat(36));
        for text in dirty
            .iter()
            .copied()
            .chain([anthropic.as_str(), github.as_str()])
        {
            assert_eq!(
                text_problems(text).len(),
                1,
                "{text}: {:?}",
                text_problems(text)
            );
        }
    }

    /// Keys are scanned as well as values; ids, hashes, working folders and transcript paths are
    /// checked wherever their key appears.
    #[test]
    fn payload_keys_ids_and_folders_are_checked() {
        let clean = serde_json::json!({
            "session_id": "00000000-0000-4000-8000-000000000001",
            "cwd": "C:\\repo",
            "transcript_path": "C:\\Users\\dev\\.claude\\projects\\C--repo\\x.jsonl",
            "agent_transcript_path": "C:\\Users\\dev\\.claude\\projects\\C--repo\\y.jsonl",
            "tool_input": {"workdir": "C:\\repo\\sub"},
            "tool_response": {"agentId": "fixtureagent0001", "harnessSectionHash": "0000"}
        });
        assert_eq!(payload_problems(&clean), Vec::<String>::new());
        let resumed = serde_json::json!({
            "cwd": "/repo",
            "transcript_path": "\\\\?\\C:\\Users\\dev\\.codex\\x.jsonl"
        });
        assert_eq!(payload_problems(&resumed), Vec::<String>::new());
        let dirty = [
            serde_json::json!({"cwd": "C:\\work\\repo"}),
            serde_json::json!({"cwd": "/repository-of-alice"}),
            serde_json::json!({"tool_input": {"workdir": "D:\\work"}}),
            serde_json::json!({"transcript_path": "D:\\logs\\x.jsonl"}),
            serde_json::json!({"agent_transcript_path": "D:\\agents\\x.jsonl"}),
            serde_json::json!({"nested": {"agent_id": "a1b2c3d4e5f6a7b8c"}}),
            serde_json::json!({"agent_id": "fixtureagent-then-anything"}),
            serde_json::json!({"turn_id": 7}),
            serde_json::json!({"files": {"/home/alice/x.rs": 1}}),
            serde_json::json!({"tool_response": {"harnessSectionHash": "9f3a"}}),
            serde_json::json!({"tool_response": "{\"path\":\"C:\\\\Users\\\\alice\\\\x\"}"}),
        ];
        for payload in dirty {
            assert_eq!(payload_problems(&payload).len(), 1, "{payload}");
        }
    }
}
