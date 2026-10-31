//! The daemon's endpoint, open to its user alone: on Unix a socket of mode 0600 in the user's run
//! folder (itself 0700); on Windows a named pipe whose access list names only the current user,
//! which also refuses clients on other machines (the pipe library's default).

use std::io;

use cortex_core::RunFiles;
use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::tokio::Listener;

/// Listens at the endpoint of `files`. Called inside the daemon's tokio runtime.
///
/// A socket file left by a daemon that crashed is replaced: the run lock, taken before this, says
/// no other daemon serves the repository.
///
/// # Errors
///
/// The endpoint cannot be named or bound, and on Windows the current user cannot be found.
#[cfg(unix)]
pub(crate) fn bind(files: &RunFiles) -> io::Result<Listener> {
    use interprocess::local_socket::{GenericFilePath, ToFsName};
    use interprocess::os::unix::local_socket::ListenerOptionsExt;
    let path = files.socket_path();
    let options = || -> io::Result<ListenerOptions<'static>> {
        Ok(ListenerOptions::new()
            .name(path.clone().to_fs_name::<GenericFilePath>()?)
            .try_overwrite(true))
    };
    match options()?.mode(0o600).create_tokio() {
        // Some systems cannot set a socket's mode before binding it. The run folder, the user's
        // alone, already keeps other users from reaching the socket.
        Err(error) if error.kind() == io::ErrorKind::Unsupported => options()?.create_tokio(),
        bound => bound,
    }
}

/// Listens at the endpoint of `files`. Called inside the daemon's tokio runtime.
///
/// # Errors
///
/// The endpoint cannot be named or bound, and on Windows the current user cannot be found.
#[cfg(windows)]
pub(crate) fn bind(files: &RunFiles) -> io::Result<Listener> {
    use interprocess::local_socket::{GenericNamespaced, ToNsName};
    use interprocess::os::windows::local_socket::ListenerOptionsExt;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    let sid = current_user_sid()?;
    // A protected access list (`P`, nothing inherited) allowing all access (`GA`) to that user.
    let sddl = widestring::U16CString::from_str(format!("D:P(A;;GA;;;{sid})"))
        .map_err(io::Error::other)?;
    ListenerOptions::new()
        .name(files.pipe_name().to_ns_name::<GenericNamespaced>()?)
        .security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
        .create_tokio()
}

/// The security identifier of the user running this process, as `whoami /user` in the Windows
/// folder reports it: `S-1-` and dashed numbers.
#[cfg(windows)]
fn current_user_sid() -> io::Result<String> {
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;
    let windows =
        std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("SystemRoot is not set"))?;
    let whoami = PathBuf::from(windows).join("System32").join("whoami.exe");
    let answer = crate::child::capture(
        Command::new(whoami).args(["/user", "/fo", "csv", "/nh"]),
        Duration::from_secs(5),
        4096,
    )?;
    if !answer.status.success() || answer.is_cut {
        return Err(io::Error::other("whoami did not name the current user"));
    }
    // One line: the account and the identifier, each in quotes, separated by a comma.
    let text = String::from_utf8_lossy(&answer.stdout);
    let sid = text
        .trim()
        .rsplit(',')
        .next()
        .map(|field| field.trim_matches('"'))
        .filter(|sid| {
            sid.starts_with("S-1-")
                && sid
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'-' || byte == b'S')
        })
        .ok_or_else(|| io::Error::other("whoami's answer holds no security identifier"))?;
    Ok(sid.to_owned())
}
