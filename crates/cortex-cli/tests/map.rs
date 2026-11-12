//! `cortex map --no-open` through the binary, against a real daemon, then the map itself through a
//! plain HTTP/1.1 client: the address it prints is served on 127.0.0.1 only, and the guard answers
//! each kind of request the way a browser, another site and another program would see it.

mod common;

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Daemon, Printed, TestResult, isolate, printed};

/// How long a request to the map may take, from connecting to its last byte.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// Runs `cortex` with `args` in the repository at `root`, `home` being the home folder.
fn cortex(root: &Path, home: &Path, args: &[&str]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).current_dir(root);
    isolate(&mut command, home);
    printed(&mut command, b"")
}

/// What the map answered: the status, the headers with lowercase names, and the body.
struct Answered {
    /// The status code.
    status: u16,
    /// Each header as `name: value`, the name in lowercase.
    headers: Vec<String>,
    /// The body.
    body: Vec<u8>,
}

/// Sends `head`, a request line and its headers without the blank line that ends them, to the map
/// at `port` on 127.0.0.1, asking it to close the connection after, and reads its whole answer.
fn send(port: u16, head: &str) -> TestResult<Answered> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut stream = TcpStream::connect_timeout(&address, ANSWER_WITHIN)?;
    stream.set_read_timeout(Some(ANSWER_WITHIN))?;
    stream.write_all(format!("{head}\r\nConnection: close\r\n\r\n").as_bytes())?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    let end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("the answer has no end of its headers")?;
    let head = String::from_utf8(bytes.get(..end).ok_or("no headers")?.to_vec())?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("the answer is empty")?;
    let status = status_line
        .strip_prefix("HTTP/1.1 ")
        .and_then(|rest| rest.get(..3))
        .ok_or("the answer has no status")?
        .parse()?;
    let headers = lines
        .map(|line| match line.split_once(':') {
            Some((name, value)) => format!("{}: {}", name.to_ascii_lowercase(), value.trim()),
            None => line.to_owned(),
        })
        .collect();
    Ok(Answered {
        status,
        headers,
        body: bytes.get(end + 4..).ok_or("no body")?.to_vec(),
    })
}

/// Whether this source tree holds a built map interface, which the daemon's build embeds.
fn has_built_map() -> bool {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist");
    std::fs::read_dir(dist).is_ok_and(|mut entries| entries.next().is_some())
}

/// `cortex map --no-open` prints the map's address and opens nothing; asked again it prints the
/// same address. The address is on 127.0.0.1 with a 64-digit token. With the token, the health
/// answers 200 with the daemon's version; a wrong token gets 404, a request naming another host
/// or sent from another site's page 403, a POST 405, and the token without its slash a 308 to the
/// address; each with an empty body, `Referrer-Policy: no-referrer` and no CORS header. The health
/// says `reading` until the daemon's first index is done and `ready` from then on. The map does not answer on the IPv6 loopback, nor on this machine's own network
/// address when it has one.
#[test]
fn the_map_is_served_on_127_0_0_1_behind_its_guard() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::write(root.join("main.py"), "def main():\n    return 0\n")?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    let asked = cortex(root, home.path(), &["map", "--no-open"])?;
    assert!(asked.is_success, "{}", asked.stderr);
    let url = asked.stdout.trim_end().to_owned();
    assert_eq!(asked.stdout, format!("{url}\n"));
    let again = cortex(root, home.path(), &["map", "--no-open"])?;
    assert_eq!(again.stdout, asked.stdout, "{}", again.stderr);
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .ok_or("the address is not on 127.0.0.1")?;
    let (port, token) = rest.split_once('/').ok_or("the address has no path")?;
    let token = token
        .strip_suffix('/')
        .ok_or("the address does not end with /")?;
    let port: u16 = port.parse()?;
    assert_eq!(token.len(), 64, "{url}");
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    let host = format!("Host: 127.0.0.1:{port}");

    // The health says `reading` until the daemon's first index is done, then `ready`.
    let until = Instant::now() + INDEXED_WITHIN;
    let (health, value) = loop {
        let health = send(port, &format!("GET /{token}/api/health HTTP/1.1\r\n{host}"))?;
        let value: serde_json::Value = serde_json::from_slice(&health.body)?;
        if value["index"] == "ready" {
            break (health, value);
        }
        let body = String::from_utf8_lossy(&health.body).into_owned();
        assert_eq!(value["index"], "reading", "{body}");
        assert!(
            Instant::now() < until,
            "the first index did not end: {body}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(health.status, 200);
    assert!(
        health
            .headers
            .contains(&"content-type: application/json".to_owned())
    );
    assert!(
        health
            .headers
            .contains(&"cache-control: no-store".to_owned())
    );
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["has_assets"], has_built_map());
    // The daemon's part comes from its own status report: it has run, its miner has a state, and
    // with history off by default its history worker is off.
    let daemon = &value["daemon"];
    assert!(daemon["uptime_ms"].as_u64().is_some(), "{value}");
    assert!(daemon["git"].as_str().is_some(), "{value}");
    assert_eq!(daemon["history"], "off", "{value}");
    assert!(daemon["errors"].is_array(), "{value}");
    let wrong_token = "0".repeat(64);
    let origin = "Origin: http://evil.example";
    for (head, status) in [
        (
            format!("GET /{wrong_token}/api/health HTTP/1.1\r\n{host}"),
            404,
        ),
        (format!("GET /api/health HTTP/1.1\r\n{host}"), 404),
        (
            format!("GET /{token}/api/health HTTP/1.1\r\nHost: evil.example:{port}"),
            403,
        ),
        (
            format!("GET /{token}/api/health HTTP/1.1\r\n{host}\r\n{origin}"),
            403,
        ),
        (
            format!("POST /{token}/api/health HTTP/1.1\r\n{host}\r\nContent-Length: 0"),
            405,
        ),
        (format!("GET /{token} HTTP/1.1\r\n{host}"), 308),
    ] {
        let answered = send(port, &head)?;
        assert_eq!(answered.status, status, "{head}");
        assert_eq!(answered.body, b"", "{head}");
        assert!(
            answered
                .headers
                .iter()
                .all(|header| !header.starts_with("access-control-")),
            "{head}"
        );
        assert!(
            answered
                .headers
                .contains(&"referrer-policy: no-referrer".to_owned()),
            "{head}"
        );
    }
    let moved = send(port, &format!("GET /{token} HTTP/1.1\r\n{host}"))?;
    assert!(moved.headers.contains(&format!("location: /{token}/")));

    let ipv6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port);
    assert!(TcpStream::connect_timeout(&ipv6, Duration::from_secs(2)).is_err());
    if let Some(own) = own_network_address() {
        let elsewhere = SocketAddr::new(own, port);
        assert!(
            TcpStream::connect_timeout(&elsewhere, Duration::from_secs(2)).is_err(),
            "the map answered on {own}"
        );
    }
    Ok(())
}

/// The address this machine would send from to the outside network, when it has one that is not
/// a loopback address. Connecting a UDP socket sends nothing: it only picks the route.
fn own_network_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // TEST-NET-1, an address reserved for documentation: nothing is ever sent to it.
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let own = socket.local_addr().ok()?.ip();
    (!own.is_loopback() && !own.is_unspecified()).then_some(own)
}

/// How long the test waits for the daemon to have indexed the repository's files.
const INDEXED_WITHIN: Duration = Duration::from_secs(60);

/// The body of the map's endpoint `path` (under the token), sent with the right `Host`.
fn api(port: u16, token: &str, path: &str) -> TestResult<Answered> {
    send(
        port,
        &format!("GET /{token}/{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}"),
    )
}

/// Through a real daemon on a repository of two files, each endpoint answers over HTTP with the
/// token: the graph lists both files sorted by path, the summary counts them, a search finds one by
/// name, the first file's panel names it, an id past the last is 404 and a search with no text 400.
#[test]
fn the_map_api_answers_from_a_real_daemons_store() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::write(root.join("main.py"), "def main():\n    return 0\n")?;
    std::fs::write(root.join("util.py"), "def helper():\n    return 1\n")?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    let asked = cortex(root, home.path(), &["map", "--no-open"])?;
    assert!(asked.is_success, "{}", asked.stderr);
    let rest = asked
        .stdout
        .trim_end()
        .strip_prefix("http://127.0.0.1:")
        .ok_or("the address is not on 127.0.0.1")?
        .trim_end_matches('/')
        .to_owned();
    let (port, token) = rest.split_once('/').ok_or("the address has no path")?;
    let port: u16 = port.parse()?;
    let until = Instant::now() + INDEXED_WITHIN;
    let graph = loop {
        let answered = api(port, token, "api/graph")?;
        let body = String::from_utf8(answered.body)?;
        if answered.status == 200 && body.contains("\"path\":\"util.py\"") {
            break body;
        }
        assert!(Instant::now() < until, "the files were not indexed: {body}");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        graph.starts_with(r#"{"version":1,"files":[{"id":0,"path":"main.py""#),
        "{graph}"
    );
    assert!(graph.contains(r#"{"id":1,"path":"util.py""#), "{graph}");
    let text = |path: &str| -> TestResult<(u16, String)> {
        let answered = api(port, token, path)?;
        Ok((answered.status, String::from_utf8(answered.body)?))
    };
    let (status, summary) = text("api/summary")?;
    assert_eq!(status, 200);
    assert!(
        summary.starts_with(r#"{"version":1,"files":2,"folders":0,"#),
        "{summary}"
    );
    let (status, found) = text("api/search?q=main")?;
    assert_eq!(status, 200);
    assert!(
        found.contains(r#"{"kind":"file","file":0,"name":"main.py","line":null}"#),
        "{found}"
    );
    let (status, panel) = text("api/file/0")?;
    assert_eq!(status, 200);
    assert!(
        panel.starts_with(r#"{"version":1,"id":0,"path":"main.py","#),
        "{panel}"
    );
    assert_eq!(text("api/file/2")?, (404, String::new()));
    assert_eq!(text("api/search")?, (400, String::new()));
    Ok(())
}
