//! Tests of [`parse_porcelain`] on porcelain output written out by hand.

use super::*;

/// Result type of these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Three commit ids.
const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
/// The second commit's id.
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
/// The third commit's id.
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// Porcelain output made of `lines`, each ended by a newline.
fn output(lines: &[&str]) -> Vec<u8> {
    let mut text = lines.join("\n");
    text.push('\n');
    text.into_bytes()
}

/// The details git prints the first time a commit appears, with `extra` lines before `filename`.
fn details(name: &str, extra: &[&str]) -> Vec<String> {
    let mut lines = vec![
        "author Ann Author".to_owned(),
        "author-mail <ann@example.com>".to_owned(),
        "author-time 1767225600".to_owned(),
        "author-tz +0000".to_owned(),
        "committer Cy Committer".to_owned(),
        "committer-mail <cy@example.com>".to_owned(),
        "committer-time 1767225600".to_owned(),
        "committer-tz +0000".to_owned(),
        "summary A summary".to_owned(),
    ];
    lines.extend(extra.iter().map(|line| (*line).to_owned()));
    lines.push(format!("filename {name}"));
    lines
}

/// Output made of header lines, details and text lines given as sections in order.
fn sections(parts: &[Vec<String>]) -> Vec<u8> {
    let lines: Vec<&str> = parts.iter().flatten().map(String::as_str).collect();
    output(&lines)
}

/// Several commits, a commit whose later lines carry a header only, a `previous` line, a
/// `boundary` line, a file name with spaces and carriage returns in the text: A owns lines 1 and 2
/// and 4, B owns 3. Counted by hand: three runs, two commits.
#[test]
fn runs_and_commits_are_read_from_headers_that_git_shortens() -> TestResult {
    let previous = format!("previous {A} src/a b.rs");
    let parts = [
        vec![format!("{A} 1 1 2")],
        details("src/a b.rs", &["boundary"]),
        vec!["\tfn one() {}\r".to_owned()],
        vec![format!("{A} 2 2"), "\t    two\r".to_owned()],
        vec![format!("{B} 5 3 1")],
        details("src/a b.rs", &[&previous]),
        vec!["\tthree".to_owned()],
        vec![format!("{A} 3 4 1"), "\tfour".to_owned()],
    ];
    let blame = parse_porcelain(&sections(&parts))?;
    let run = |start, end, source_start, oid: &str| BlameRun {
        start,
        end,
        oid: oid.to_owned(),
        source_start,
    };
    assert_eq!(
        blame.runs,
        [run(1, 2, 1, A), run(3, 3, 5, B), run(4, 4, 3, A)]
    );
    assert_eq!(blame.commits.len(), 2);
    assert_eq!(
        blame.commits.get(A),
        Some(&BlamedCommit {
            previous: None,
            is_boundary: true,
        })
    );
    assert_eq!(
        blame.commits.get(B),
        Some(&BlamedCommit {
            previous: Some(Previous {
                oid: A.to_owned(),
                path: "src/a b.rs".to_owned(),
            }),
            is_boundary: false,
        })
    );
    Ok(())
}

/// Lines of details this reader does not know, git's `ignored` and `unblamable` after a header
/// that is not a commit's first, and a tab alone for an empty line of text are skipped.
#[test]
fn details_that_are_not_known_are_skipped() -> TestResult {
    let parts = [
        vec![format!("{C} 1 1 1")],
        details("a.rs", &["a-future-key with a value", "unblamable"]),
        vec!["\t".to_owned()],
        vec![
            format!("{C} 2 2 1"),
            "ignored".to_owned(),
            "\ttext".to_owned(),
        ],
    ];
    let blame = parse_porcelain(&sections(&parts))?;
    assert_eq!(
        blame.runs,
        [BlameRun {
            start: 1,
            end: 2,
            oid: C.to_owned(),
            source_start: 1,
        }]
    );
    Ok(())
}

/// A path in quotes is undone as git quotes it: a byte in octal, a quote and a backslash; a path
/// with no quotes is kept as it is, even with a backslash in it.
#[test]
fn a_quoted_previous_path_is_unquoted() -> TestResult {
    let cases = [
        (r#""d\303\251 \"x\" \\ \t.rs""#, "d\u{e9} \"x\" \\ \t.rs"),
        ("plain name.rs", "plain name.rs"),
        (r"back\slash.rs", r"back\slash.rs"),
    ];
    for (written, expected) in cases {
        let previous = format!("previous {A} {written}");
        let parts = [
            vec![format!("{B} 1 1 1")],
            details("f", &[&previous]),
            vec!["\ttext".to_owned()],
        ];
        let blame = parse_porcelain(&sections(&parts))?;
        let found = blame
            .commits
            .get(B)
            .and_then(|commit| commit.previous.as_ref());
        assert_eq!(found.map(|previous| previous.path.as_str()), Some(expected));
    }
    Ok(())
}

/// Lines that are not next to each other are runs of their own even for one commit; an id of 64
/// digits is read; nothing at all is an empty blame.
#[test]
fn gaps_split_runs_and_long_ids_are_read() -> TestResult {
    let long = "d".repeat(64);
    let parts = [
        vec![format!("{long} 1 1 1")],
        details("f", &[]),
        vec!["\tone".to_owned()],
        vec![format!("{long} 3 3 1"), "\tthree".to_owned()],
    ];
    let blame = parse_porcelain(&sections(&parts))?;
    assert_eq!(blame.runs.len(), 2);
    assert_eq!((blame.runs[0].start, blame.runs[0].end), (1, 1));
    assert_eq!((blame.runs[1].start, blame.runs[1].end), (3, 3));

    assert_eq!(parse_porcelain(b"")?, Blame::default());
    Ok(())
}

/// Output that is not porcelain is an error, never a panic: each case breaks one rule.
#[test]
fn malformed_output_is_an_error() {
    let first = format!("{A} 1 1 1");
    let second = format!("{A} 2 2 1");
    let two_fields = format!("{A} 1");
    let non_hex = format!("{} 1 1 1", "z".repeat(40));
    let short_id = format!("{} 1 1 1", "a".repeat(39));
    let zero_line = format!("{A} 1 0 1");
    let word_line = format!("{A} 1 x 1");
    let no_path = format!("previous {A}");
    let escape = format!("previous {A} \"bad\\q.rs\"");
    let octal = format!("previous {A} \"bad\\39x.rs\"");
    let lone = format!("previous {A} \"bad\\\"");
    let file = "filename f";
    let text = "\tx";
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("a header of two fields", vec![two_fields.as_str(), text]),
        (
            "an id that is not hexadecimal",
            vec![non_hex.as_str(), file, text],
        ),
        (
            "an id that is too short",
            vec![short_id.as_str(), file, text],
        ),
        (
            "a line number of zero",
            vec![zero_line.as_str(), file, text],
        ),
        (
            "a line number that is a word",
            vec![word_line.as_str(), file, text],
        ),
        ("no filename line", vec![first.as_str(), "summary s", text]),
        (
            "a previous line with no path",
            vec![first.as_str(), no_path.as_str(), file, text],
        ),
        (
            "a previous line with an id that is not one",
            vec![first.as_str(), "previous nothing f", file, text],
        ),
        (
            "an escape git does not write",
            vec![first.as_str(), escape.as_str(), file, text],
        ),
        (
            "an octal escape with a digit of 9",
            vec![first.as_str(), octal.as_str(), file, text],
        ),
        (
            "a backslash at the end of a quoted path",
            vec![first.as_str(), lone.as_str(), file, text],
        ),
        ("text that never comes", vec![first.as_str(), file]),
        ("text first", vec![text]),
        (
            "line numbers that do not rise",
            vec![
                first.as_str(),
                file,
                text,
                second.as_str(),
                text,
                first.as_str(),
                text,
            ],
        ),
    ];
    for (name, lines) in cases {
        let result = parse_porcelain(&output(&lines));
        assert!(
            matches!(result, Err(BlameError::Malformed(_))),
            "{name}: {result:?}"
        );
    }

    // A header, or the path of a previous commit, that is not UTF-8 cannot be read.
    let mut bad_header = vec![0xff_u8; 40];
    bad_header.extend_from_slice(b" 1 1 1\nfilename f\n\tx\n");
    let mut bad_path = format!("{first}\nprevious {A} ").into_bytes();
    bad_path.extend_from_slice(b"\xff.rs\nfilename f\n\tx\n");
    for (name, bytes) in [("a header", bad_header), ("a previous path", bad_path)] {
        let result = parse_porcelain(&bytes);
        assert!(
            matches!(result, Err(BlameError::Malformed(_))),
            "{name}: {result:?}"
        );
    }
}

/// An author's name and a summary are bytes as the commit recorded them, and one that is not UTF-8
/// does not stop the blame: the lines are read and the commit is listed.
#[test]
fn details_that_are_not_kept_may_be_any_bytes() -> TestResult {
    let mut bytes = format!("{A} 1 1 1\n").into_bytes();
    bytes.extend_from_slice(b"author \xff\xfe Author\nauthor-mail <a@example.com>\n");
    bytes.extend_from_slice(b"summary caf\xe9 fix\ncommitter \xc3\n");
    bytes.extend_from_slice(b"filename f\n\ttext\n");
    let blame = parse_porcelain(&bytes)?;
    assert_eq!(blame.runs.len(), 1);
    assert_eq!(blame.runs[0].oid, A);
    assert_eq!(blame.commits.len(), 1);
    Ok(())
}

/// Lines are one run only when the final file's numbers and the commit's own both run on. Counted
/// by hand from the headers: B's lines 1 and 2 of the final file stood at 7 and 8 in B's file (one
/// run); line 3 stood at 2, which does not follow 8, so it starts a run that line 4, which stood
/// at 3, continues; line 5 stood at 9 and starts a third; line 6 is A's, from line 3 of A's file.
#[test]
fn a_run_needs_both_numbers_to_run_on() -> TestResult {
    let parts = [
        vec![format!("{B} 7 1 2")],
        details("f", &[]),
        vec!["\tone".to_owned()],
        vec![format!("{B} 8 2"), "\ttwo".to_owned()],
        vec![format!("{B} 2 3 1"), "\tthree".to_owned()],
        vec![format!("{B} 3 4 1"), "\tfour".to_owned()],
        vec![format!("{B} 9 5 1"), "\tfive".to_owned()],
        vec![format!("{A} 3 6 1")],
        details("f", &[]),
        vec!["\tsix".to_owned()],
    ];
    let blame = parse_porcelain(&sections(&parts))?;
    let found: Vec<(u32, u32, u32, &str)> = blame
        .runs
        .iter()
        .map(|run| (run.start, run.end, run.source_start, run.oid.as_str()))
        .collect();
    assert_eq!(
        found,
        [(1, 2, 7, B), (3, 4, 2, B), (5, 5, 9, B), (6, 6, 3, A)]
    );
    Ok(())
}
