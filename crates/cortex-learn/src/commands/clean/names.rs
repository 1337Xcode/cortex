//! Whether a name is a credential's: an option's (`--api-key`), a variable's (`DB_PASSWORD`), a
//! property's (`db.password`, `ApiToken`) or a query parameter's (`access_token`). A name is
//! compared in lower case with `-`, `_` and `.` taken out, so `--api-key`, `API_KEY`, `apiKey`
//! and `api.key` all end in `apikey`; it is a credential's when it ends in one of
//! [`CREDENTIAL_ENDS`] and does not start `no-` or `no_`, which turns an option off.
//!
//! The ends are kept short on purpose: `key` takes in `--sort-key` too, whose value is then
//! redacted needlessly, which costs a handoff one argument; a credential missed costs the user
//! the credential.

/// The ends of credentials' names, once lower-cased and squeezed.
const CREDENTIAL_ENDS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "passphrase",
    "pass",
    "pwd",
    "key",
    "auth",
    "authorization",
    "credential",
    "credentials",
    "cookie",
];

/// Whether `name` names a credential.
pub(super) fn is_credential(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("no-") || lower.starts_with("no_") {
        return false;
    }
    let squeezed: String = lower
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | '.'))
        .collect();
    CREDENTIAL_ENDS.iter().any(|end| squeezed.ends_with(end))
}
