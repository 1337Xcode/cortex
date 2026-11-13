//! `cortex map`: asks the repository's daemon for the map's address, which serves the map from then
//! on, prints it and opens it in the user's browser. The daemon is started when none runs.
//!
//! The address holds the map's access token: whoever has it can read the map, so it is printed
//! only to the user who asked, and the map listens on 127.0.0.1 alone.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cortex_core::ipc::{MapAnswer, RequestBody, ResponseBody};
use cortex_core::{RepoId, UserFolder, create_private_folder, open_private_file};

use crate::upgrade::LOOK_EVERY;
use crate::{output, repository, upgrade};

/// How long the daemon has to answer, a daemon started by this command included: it answers once
/// it listens and has read its configuration.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// How long the program that opens the browser is waited for. It hands the address to the browser
/// and ends; one still running after this is left to finish on its own.
const OPEN_WITHIN: Duration = Duration::from_secs(10);

/// The arguments of `cortex map`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Print the address without opening it.
    #[arg(long)]
    no_open: bool,
}

/// Prints the map's address, then opens it unless asked not to.
///
/// # Errors
///
/// A root that is not a folder, no home folder, a daemon that cannot be started, does not answer
/// or cannot serve the map, an address that is not the map's, or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let (root, _) = repository::root_and_worktree(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let asked = upgrade::ask_started(
        &root,
        &user,
        &RequestBody::MapAddress,
        Instant::now() + ANSWER_WITHIN,
    );
    if let Some(old) = &asked.replaced {
        output::note(&format!("replaced a daemon of Cortex {old}"))?;
    }
    let answer = asked
        .answer
        .with_context(|| format!("asking the daemon of {}", root.display()))?;
    let (url, has_assets) = match answer.body {
        ResponseBody::Map(MapAnswer::Serving { url, has_assets }) => (url, has_assets),
        ResponseBody::Map(MapAnswer::BindFailed { port, cause }) => {
            bail!("the map could not listen on 127.0.0.1 port {port}: {cause}")
        }
        ResponseBody::Error { message } => bail!("{message}"),
        other => bail!("the daemon answered with {other:?}"),
    };
    if !is_map_address(&url) {
        bail!(
            "the daemon gave an address that is not the map's on 127.0.0.1; it was not printed or \
             opened"
        );
    }
    output::line(&url)?;
    if !has_assets {
        output::note("this build of Cortex was made without the map's files; the page says so")?;
    }
    if args.no_open {
        return Ok(());
    }
    let page = user.run_files(&RepoId::of(&root)).map_page_path();
    if let Err(error) = write_opening_page(&page, &url).and_then(|()| open(&page)) {
        output::note(&format!(
            "the browser was not opened ({error:#}); open the address above yourself"
        ))?;
    }
    Ok(())
}

/// How many hexadecimal digits the map's token is written with.
const TOKEN_DIGITS: usize = 64;

/// Whether `url` is exactly an address the map is served at: `http://127.0.0.1:`, a port from 1 to
/// 65535 written without a leading zero, `/`, the token's 64 lowercase hexadecimal digits and `/`.
/// Only such an address is printed and handed to the browser's opener: it starts with no `-` an
/// opener would read as an option, and it names nothing but the map on this machine.
fn is_map_address(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let Some((port, path)) = rest.split_once('/') else {
        return false;
    };
    let is_port = !port.starts_with('0') && port.parse::<u16>().is_ok_and(|number| number > 0);
    let Some(token) = path.strip_suffix('/') else {
        return false;
    };
    is_port
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && token.len() == TOKEN_DIGITS
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The page that sends the browser on to `url`, a map address [`is_map_address`] accepted, which
/// holds no character an HTML attribute or text would need escaped. It sends no `Referer` on.
fn opening_page(url: &str) -> String {
    format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<meta name=\"referrer\" content=\"no-referrer\">\n\
         <meta http-equiv=\"refresh\" content=\"0;url={url}\">\n<title>Cortex map</title>\n\
         <a href=\"{url}\">Open the map</a>\n"
    )
}

/// Writes the page that sends the browser on to `url` at `path`, in the user's run folder,
/// readable by the user alone: the opener is then given the page's path, and the address with its
/// token is on no command line, where on Linux every user of the machine could read it (and a
/// browser the opener starts keeps it in its own for its whole life).
///
/// # Errors
///
/// The folder or the page cannot be created or written.
fn write_opening_page(path: &Path, url: &str) -> anyhow::Result<()> {
    if let Some(folder) = path.parent() {
        create_private_folder(folder).with_context(|| format!("creating {}", folder.display()))?;
    }
    let mut file =
        open_private_file(path).with_context(|| format!("opening {}", path.display()))?;
    file.set_len(0)
        .and_then(|()| file.write_all(opening_page(url).as_bytes()))
        .with_context(|| format!("writing {}", path.display()))
}

/// Opens the page at `page` in the user's browser with the system's own opener, given its path as
/// one argument, never through a shell.
///
/// # Errors
///
/// The opener cannot be found or started, or, but on Windows, ends with a failure. Explorer's exit
/// status says nothing: it ends with 1 when it opened the page.
fn open(page: &Path) -> anyhow::Result<()> {
    let mut opener = opener()?;
    let mut child = opener
        .arg(page)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the program that opens the browser")?;
    let deadline = Instant::now() + OPEN_WITHIN;
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .context("waiting for the browser's opener")?
        {
            if status.success() || cfg!(windows) {
                return Ok(());
            }
            bail!("the program that opens the browser ended with {status}");
        }
        std::thread::sleep(LOOK_EVERY);
    }
    Ok(())
}

/// The program that opens an address in the user's browser on macOS.
#[cfg(target_os = "macos")]
fn opener() -> anyhow::Result<Command> {
    Ok(Command::new("open"))
}

/// The program that opens an address in the user's browser on Windows: Explorer, from the
/// Windows folder rather than wherever the search path would find one.
#[cfg(windows)]
fn opener() -> anyhow::Result<Command> {
    let windows = std::env::var_os("SystemRoot").context("SystemRoot is not set")?;
    Ok(Command::new(PathBuf::from(windows).join("explorer.exe")))
}

/// The program that opens an address in the user's browser on Linux and the other Unix systems.
#[cfg(all(unix, not(target_os = "macos")))]
fn opener() -> anyhow::Result<Command> {
    Ok(Command::new("xdg-open"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The addresses the map is served at are taken; any other, however close, is refused.
    #[test]
    fn only_the_map_s_address_is_taken() {
        let token = "0123456789abcdef".repeat(4);
        for taken in [
            format!("http://127.0.0.1:8123/{token}/"),
            format!("http://127.0.0.1:1/{token}/"),
            format!("http://127.0.0.1:65535/{token}/"),
        ] {
            assert!(is_map_address(&taken), "{taken}");
        }
        let upper = token.to_uppercase();
        let short = &token[1..];
        for refused in [
            format!("http://localhost:8123/{token}/"),
            format!("https://127.0.0.1:8123/{token}/"),
            format!("http://127.0.0.1:8123/{token}"),
            format!("http://127.0.0.1:8123/{token}/x"),
            format!("http://127.0.0.1:8123/{upper}/"),
            format!("http://127.0.0.1:8123/{short}/"),
            format!("http://127.0.0.1:8123/{token}0/"),
            format!("http://127.0.0.1:0/{token}/"),
            format!("http://127.0.0.1:08123/{token}/"),
            format!("http://127.0.0.1:65536/{token}/"),
            format!("http://127.0.0.1:+8123/{token}/"),
            format!("http://127.0.0.1/{token}/"),
            format!("http://127.0.0.1:8123@evil.example/{token}/"),
            format!("-http://127.0.0.1:8123/{token}/"),
            "--new-window".to_owned(),
            String::new(),
        ] {
            assert!(!is_map_address(&refused), "{refused}");
        }
    }

    /// The page sends the browser on to the address and holds it in full, in a refresh and a link,
    /// sending no `Referer`; written again, it holds the newer address alone (a longer page before
    /// is cut, not left trailing), and on Unix only the user may read it.
    #[test]
    fn the_opening_page_holds_the_address_for_the_user_alone()
    -> Result<(), Box<dyn std::error::Error>> {
        let token = "0123456789abcdef".repeat(4);
        let url = format!("http://127.0.0.1:8123/{token}/");
        let folder = tempfile::tempdir()?;
        let page = folder.path().join("run").join("id.map.html");
        write_opening_page(&page, &format!("{url}{}", "x".repeat(400)))?;
        write_opening_page(&page, &url)?;
        let text = std::fs::read_to_string(&page)?;
        assert_eq!(text, opening_page(&url));
        assert!(text.contains(&format!("content=\"0;url={url}\"")), "{text}");
        assert!(text.contains(&format!("<a href=\"{url}\">")), "{text}");
        assert!(
            text.contains("<meta name=\"referrer\" content=\"no-referrer\">"),
            "{text}"
        );
        assert!(!text.contains("xxxx"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&page)?.permissions().mode() & 0o777,
                0o600
            );
        }
        Ok(())
    }
}
