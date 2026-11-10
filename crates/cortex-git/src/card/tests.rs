//! Tests of [`read_card`]: commit messages written out by hand, with the card each one must give
//! typed beside it.

use super::*;

/// Result type of these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A commit id.
const OID: &str = "0123456789abcdef0123456789abcdef01234567";

/// A parent's id.
const PARENT: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// A second parent's id.
const OTHER_PARENT: &str = "fedcba9876543210fedcba9876543210fedcba98";

/// The text `git show -s` prints with [`SHOW_FORMAT`] for a commit by `author` and `committer`.
fn show_by(author: &str, committer: &str, subject: &str, body: &str) -> String {
    format!("{OID}\n{PARENT}\n1767225600\n{author}\n{committer}\n{subject}\n{body}\n")
}

/// The text for a commit by a person.
fn show(subject: &str, body: &str) -> String {
    show_by("Ann Author", "Cy Committer", subject, body)
}

/// A context for an ordinary commit: two paths changed, a limit of 30, not cosmetic, no bot
/// patterns.
fn plain() -> CardContext<'static> {
    CardContext {
        changed_paths: ChangedPaths::Count(2),
        max_files: 30,
        is_cosmetic: false,
        bot_patterns: &[],
    }
}

/// The card for a commit by a person in an ordinary context.
fn card_of(subject: &str, body: &str) -> Result<ChangeCard, CardError> {
    read_card(&show(subject, body), &plain())
}

/// The fields of the header: the id, two parents, the time, and no reason for a message of one
/// line.
#[test]
fn the_header_gives_the_id_the_parents_and_the_time() -> TestResult {
    let text = format!("{OID}\n{PARENT} {OTHER_PARENT}\n1767225600\nA\nC\nAdd the parser\n\n");
    let card = read_card(&text, &plain())?;
    assert_eq!(card.oid, OID);
    assert_eq!(card.parents, [PARENT, OTHER_PARENT]);
    assert_eq!(card.time_s, 1_767_225_600);
    assert_eq!(card.subject, "Add the parser");
    assert_eq!(card.reason, None);
    assert_eq!(card.kind, ChangeKind::Normal);
    assert_eq!(card.author, AuthorKind::Human);
    Ok(())
}

/// A root commit has no parents: the second line is empty.
#[test]
fn a_root_commit_has_no_parents() -> TestResult {
    let text = format!("{OID}\n\n1767225600\nA\nC\nFirst\n\n");
    assert_eq!(read_card(&text, &plain())?.parents, Vec::<String>::new());
    Ok(())
}

/// Malformed text is an error that says which part: a missing line, an id that is not
/// hexadecimal or has the wrong length, a time that is not a number.
#[test]
fn malformed_text_is_an_error() {
    let context = plain();
    assert_eq!(read_card("", &context), Err(CardError::MissingLine("id")));
    assert_eq!(
        read_card(&format!("{OID}\n{PARENT}\n1\nA\nC\n"), &context),
        Err(CardError::MissingLine("subject"))
    );
    assert_eq!(
        read_card(&format!("xyz\n{PARENT}\n1\nA\nC\nS\n"), &context),
        Err(CardError::BadId)
    );
    assert_eq!(
        read_card(&format!("{OID}\n{PARENT} 1234\n1\nA\nC\nS\n"), &context),
        Err(CardError::BadId)
    );
    assert_eq!(
        read_card(&format!("{OID}\n{PARENT}\nyesterday\nA\nC\nS\n"), &context),
        Err(CardError::BadTime)
    );
    let bad_glob = ["[".to_owned()];
    let context = CardContext {
        bot_patterns: &bad_glob,
        ..plain()
    };
    assert_eq!(
        read_card(&show("S", ""), &context),
        Err(CardError::BadBotPattern("[".to_owned()))
    );
}

/// The four forms of a pull request number, each typed by hand.
#[test]
fn a_pull_request_number_comes_from_each_form() -> TestResult {
    let squash = card_of("Fix the parser (#123)", "")?;
    assert_eq!(squash.pull_request, Some(123));

    let merge = card_of(
        "Merge pull request #45 from owner/branch",
        "Handle empty input",
    )?;
    assert_eq!(merge.pull_request, Some(45));
    assert_eq!(merge.reason.as_deref(), Some("Handle empty input"));

    let gitlab = card_of(
        "Merge branch 'feature' into 'main'",
        "Add retries\n\nSee merge request group/sub.project!77",
    )?;
    assert_eq!(gitlab.pull_request, Some(77));

    let bang = card_of("Add retries (!9)", "")?;
    assert_eq!(bang.pull_request, Some(9));

    assert_eq!(card_of("Add retries", "")?.pull_request, None);
    assert_eq!(card_of("Fix #5 properly", "")?.pull_request, None);
    Ok(())
}

/// When a message matches several forms the first of: squash, merge, GitLab, bang wins; a number
/// too long for 64 bits matches nothing.
#[test]
fn a_pull_request_form_that_comes_first_wins() -> TestResult {
    let both = card_of("Merge pull request #5 from a/b (#6)", "")?;
    assert_eq!(both.pull_request, Some(6));
    let gitlab_over_bang = card_of("Add retries (!3)", "See merge request g/p!8")?;
    assert_eq!(gitlab_over_bang.pull_request, Some(8));
    let huge = card_of("Add retries (#99999999999999999999999)", "")?;
    assert_eq!(huge.pull_request, None);
    Ok(())
}

/// Each keyword and its variants, with and without a colon, in any case, in the subject or the
/// body, names its issue as written.
#[test]
fn every_issue_keyword_names_its_issue() -> TestResult {
    let cases = [
        ("close #1", "#1"),
        ("Closes #2", "#2"),
        ("closed #3", "#3"),
        ("fix #4", "#4"),
        ("Fixes #5", "#5"),
        ("fixed #6", "#6"),
        ("resolve #7", "#7"),
        ("Resolves #8", "#8"),
        ("RESOLVED #9", "#9"),
        ("fixes: #10", "#10"),
        ("Closes:  #11", "#11"),
    ];
    for (text, expected) in cases {
        assert_eq!(card_of("Change it", text)?.issues, [expected], "{text}");
        assert_eq!(card_of(text, "")?.issues, [expected], "{text}");
    }
    Ok(())
}

/// Words that only contain a keyword, a keyword with no number, and a number with no keyword name
/// no issue.
#[test]
fn text_that_is_no_reference_names_no_issue() -> TestResult {
    for text in [
        "prefix #3",
        "fixing #4",
        "unclosed #5",
        "closes the loop",
        "see #6",
    ] {
        assert_eq!(
            card_of("Change it", text)?.issues,
            Vec::<String>::new(),
            "{text}"
        );
    }
    Ok(())
}

/// A reference to another repository is kept as written.
#[test]
fn a_reference_to_another_repository_is_kept_whole() -> TestResult {
    let card = card_of("Change it", "Fixes owner/some-repo.rs#12\nCloses #13")?;
    assert_eq!(card.issues, ["owner/some-repo.rs#12", "#13"]);
    Ok(())
}

/// At most three issues, in the order they appear, and an issue named twice once.
#[test]
fn at_most_three_issues_without_repeats() -> TestResult {
    let many = card_of("Fixes #1", "closes #2, resolves #3 and fixes #4")?;
    assert_eq!(many.issues, ["#1", "#2", "#3"]);
    let repeated = card_of(
        "Fixes #1",
        "closes #1, resolves #2, fixes #1, closes #3, fixes #4",
    )?;
    assert_eq!(repeated.issues, ["#1", "#2", "#3"]);
    Ok(())
}

/// A body of trailers only gives no reason; each trailer of the known keys and a key of the shape
/// of one is skipped.
#[test]
fn a_body_of_trailers_gives_no_reason() -> TestResult {
    let body = "Signed-off-by: Ann <ann@example.com>\n\
        Co-authored-by: Bo <bo@example.com>\n\
        Reviewed-by: Cy <cy@example.com>\n\
        Change-Id: I0123abc\n\
        Cc: dev@example.com\n\
        X-Internal-Ticket: 55\n\
        Fixes: #4";
    assert_eq!(card_of("Change it", body)?.reason, None);
    Ok(())
}

/// The reason is the first line that says something: after a blank line, trailers, a link alone
/// (bare or in angle brackets) and bullets of file names, and it is trimmed.
#[test]
fn the_reason_is_the_first_line_that_says_something() -> TestResult {
    let body = "\n\
        Signed-off-by: Ann <ann@example.com>\n\
        https://example.com/issues/12\n\
        <https://example.com/issues/13>\n\
        - src/lib.rs, src/card.rs\n\
        * Cargo.toml\n\
        +  `a.txt` `b/c`\n\
        \u{2022} README.md\n\
        \x20  The parser looped on empty input   \n\
        Later lines are not read";
    let card = card_of("Change it", body)?;
    assert_eq!(
        card.reason.as_deref(),
        Some("The parser looped on empty input")
    );
    Ok(())
}

/// A line that only repeats what the subject or git said is no reason: an issue keyword and
/// references alone (with a colon, a repository, a list, a full stop), git's revert line and
/// cherry-pick line, and the subject itself, as it is or as a bullet. The next line that says
/// something is.
#[test]
fn a_line_that_adds_nothing_is_skipped() -> TestResult {
    let skipped = [
        "Fixes #12",
        "closes: owner/repo#3, #4.",
        "Resolves #1 and #2",
        "This reverts commit 0123456789abcdef0123456789abcdef01234567.",
        "(cherry picked from commit 0123456789abcdef0123456789abcdef01234567)",
        "Add the parser",
        "- Add the parser",
        "*   Add the parser",
    ];
    for line in skipped {
        let body = format!("{line}\n\nThe parser looped on empty input");
        let card = card_of("Add the parser", &body)?;
        assert_eq!(
            card.reason.as_deref(),
            Some("The parser looped on empty input"),
            "{line}"
        );
    }
    for line in skipped {
        assert_eq!(card_of("Add the parser", line)?.reason, None, "{line}");
    }
    Ok(())
}

/// A line that begins like those but says more is a reason: a keyword with words, a reference
/// with words, text after a bullet that the subject is not, a revert line of another shape.
#[test]
fn a_line_that_says_more_than_a_keyword_is_a_reason() -> TestResult {
    let cases = [
        "Fixes the crash from #12",
        "Fixes #12 by checking for empty input",
        "- Add the parser for tables",
        "Add the parser again",
        "Revert of the earlier change",
    ];
    for line in cases {
        assert_eq!(
            card_of("Add the parser", line)?.reason.as_deref(),
            Some(line),
            "{line}"
        );
    }
    Ok(())
}

/// Lines that look like trailers or lists but say something are reasons: a key without a hyphen
/// that is not known, a link in a sentence, a bullet with words, a bullet of a name and a word,
/// a version number.
#[test]
fn a_line_that_says_something_is_a_reason() -> TestResult {
    let cases = [
        "Why: the cache never expired",
        "See https://example.com/a for the cause",
        "- fix the loop in the parser",
        "- update README.md",
        "- 1.2 is out",
        "* e.g. the old path",
    ];
    for line in cases {
        assert_eq!(
            card_of("Change it", line)?.reason.as_deref(),
            Some(line),
            "{line}"
        );
    }
    Ok(())
}

/// The reason keeps 100 characters; the subject 72.
#[test]
fn the_reason_and_the_subject_are_trimmed() -> TestResult {
    let long_subject = "x".repeat(150);
    let card = card_of(&long_subject, &"y".repeat(150))?;
    assert_eq!(card.reason, Some("y".repeat(100)));
    assert_eq!(card.subject, "x".repeat(72));
    Ok(())
}

/// The subject is cut on a character boundary: 71 letters then two two-byte characters keep 71
/// letters and one of them; eighty three-byte characters keep seventy-two; a space the cut leaves
/// at the end is dropped.
#[test]
fn the_subject_is_cut_on_a_character_boundary() -> TestResult {
    let two_bytes = format!("{}\u{e9}\u{e9}", "a".repeat(71));
    let card = card_of(&two_bytes, "")?;
    assert_eq!(card.subject, format!("{}\u{e9}", "a".repeat(71)));
    assert_eq!(card.subject.len(), 73);

    let three_bytes = "\u{65e5}".repeat(80);
    assert_eq!(card_of(&three_bytes, "")?.subject, "\u{65e5}".repeat(72));

    let exact = "b".repeat(72);
    assert_eq!(card_of(&exact, "")?.subject, exact);

    let spaced = format!("{} tail", "c".repeat(71));
    assert_eq!(card_of(&spaced, "")?.subject, "c".repeat(71));
    Ok(())
}

/// An empty body gives no reason and no issue; carriage returns at the ends of lines are not part
/// of any value.
#[test]
fn an_empty_body_and_carriage_returns_change_nothing() -> TestResult {
    let empty = card_of("Add the parser", "")?;
    assert_eq!((empty.reason, empty.issues), (None, Vec::new()));

    let text = format!(
        "{OID}\r\n{PARENT}\r\n1767225600\r\nA\r\nC\r\nAdd the parser (#7)\r\n\r\n\
        Handle empty input\r\n\r\nFixes #8\r\nSigned-off-by: dependabot[bot] <d@x>\r\n"
    );
    let card = read_card(&text, &plain())?;
    assert_eq!(card.oid, OID);
    assert_eq!(card.parents, [PARENT]);
    assert_eq!(card.subject, "Add the parser (#7)");
    assert_eq!(card.pull_request, Some(7));
    assert_eq!(card.reason.as_deref(), Some("Handle empty input"));
    assert_eq!(card.issues, ["#8"]);
    assert_eq!(card.author, AuthorKind::Human);
    Ok(())
}

/// A `[bot]` name for the author makes a bot, in any case; so does a caller's pattern, without
/// case; a person with a plain name and a Co-authored-by bot is not one.
#[test]
fn a_bot_is_known_by_its_name_or_a_pattern() -> TestResult {
    let patterns = ["ci-*".to_owned(), "renovate".to_owned()];
    let context = CardContext {
        bot_patterns: &patterns,
        ..plain()
    };
    let by = |author: &str, committer: &str, body: &str| -> Result<AuthorKind, CardError> {
        Ok(read_card(&show_by(author, committer, "Update it", body), &context)?.author)
    };
    assert_eq!(by("dependabot[bot]", "Cy", "")?, AuthorKind::Bot);
    assert_eq!(by("GitHub-Actions[BOT]", "Cy", "")?, AuthorKind::Bot);
    assert_eq!(by("CI-runner-7", "Cy", "")?, AuthorKind::Bot);
    assert_eq!(by("Renovate", "Cy", "")?, AuthorKind::Bot);
    assert_eq!(by("Ann", "Cy", "")?, AuthorKind::Human);
    assert_eq!(by("Renovate Fan", "Cy", "")?, AuthorKind::Human);
    assert_eq!(
        by("Ann", "Cy", "Co-authored-by: dependabot[bot] <d@x.example>")?,
        AuthorKind::Human
    );
    Ok(())
}

/// A bot that lands or rebases a person's commit does not make it a bot's: with a named author the
/// committer, and a sign-off by a bot, do not count, so the person's reason stays. They count
/// when the author's name is empty.
#[test]
fn a_bot_that_lands_a_persons_commit_does_not_hide_it() -> TestResult {
    let patterns = ["ci-*".to_owned()];
    let context = CardContext {
        bot_patterns: &patterns,
        ..plain()
    };
    let read = |author: &str, committer: &str, body: &str| {
        read_card(
            &show_by(author, committer, "Fix the parser", body),
            &context,
        )
    };
    let landed = read("Ann", "mergify[bot]", "Handle empty input")?;
    assert_eq!(landed.author, AuthorKind::Human);
    assert_eq!(landed.kind, ChangeKind::Normal);
    assert_eq!(landed.reason.as_deref(), Some("Handle empty input"));

    let signed = read("Ann", "Cy", "Signed-off-by: renovate[bot] <r@x.example>")?;
    assert_eq!(signed.author, AuthorKind::Human);

    let nameless = read("", "mergify[bot]", "")?;
    assert_eq!(
        (nameless.author, nameless.kind),
        (AuthorKind::Bot, ChangeKind::Bot)
    );
    let nameless_signed = read("", "Cy", "Signed-off-by: ci-bot <r@x.example>")?;
    assert_eq!(nameless_signed.author, AuthorKind::Bot);
    let nameless_person = read("", "Cy", "")?;
    assert_eq!(nameless_person.author, AuthorKind::Human);

    let by_bot = read("dependabot[bot]", "Cy", "")?;
    assert_eq!(
        (by_bot.author, by_bot.kind),
        (AuthorKind::Bot, ChangeKind::Bot)
    );
    Ok(())
}

/// No name reaches the card: neither the author's nor the committer's appears anywhere in it.
#[test]
fn no_name_is_kept() -> TestResult {
    let text = show_by(
        "Zelda Zzyzx",
        "Quentin Qux",
        "Add the parser",
        "Handle empty input",
    );
    let card = read_card(&text, &plain())?;
    let printed = format!("{card:?}");
    assert!(
        !printed.contains("Zelda") && !printed.contains("Quentin"),
        "{printed}"
    );
    Ok(())
}

/// Subjects of housekeeping are trivial: each phrase of the table with its inflections, after a
/// conventional commit prefix or without one, in any case; a `style` type, a `release` or `deps`
/// scope.
#[test]
fn housekeeping_subjects_are_trivial() -> TestResult {
    let subjects = [
        "WIP: parser",
        "[WIP] parser",
        "wip",
        "Fix typos",
        "fix typo in the readme",
        "Typos in the docs",
        "fmt",
        "Format the sources",
        "Formatting",
        "Reformat the tables",
        "Reformatted the header",
        "Fix formatting",
        "lint fixes",
        "Linting",
        "Linter cleanup",
        "Fix linter warnings",
        "Fix lint warnings",
        "Whitespace cleanup",
        "Fix whitespace",
        "Run rustfmt",
        "run Prettier over the docs",
        "Run black",
        "Run gofmt",
        "Bump serde from 1.0.1 to 1.0.2",
        "chore: Bump serde from 1.0.1 to 1.0.2",
        "Release 1.4.0",
        "Release v2.0.1",
        "Merge branch 'main' into feature",
        "Merge remote-tracking branch 'origin/main'",
        "Merge tag 'v1.2'",
        "Revert \"Revert \"Add the parser\"\"",
        "style: align the columns",
        "style(parser)!: align the columns",
        "chore(release): 1.4.0",
        "build(deps): update the parser crate",
        "fix(deps): move the parser crate",
        "chore: wip on the parser",
        "fix: typo in the readme",
        "docs: fix whitespace",
        "refactor(release): split the notes step",
    ];
    for subject in subjects {
        assert_eq!(card_of(subject, "")?.kind, ChangeKind::Trivial, "{subject}");
    }
    Ok(())
}

/// Real changes that mention a housekeeping word are normal: the word does not begin the subject,
/// or is another word, or the phrase asks for more (a release is a version).
#[test]
fn changes_that_mention_housekeeping_words_are_normal() -> TestResult {
    let subjects = [
        "Fix date format in invoice header",
        "Validate email format",
        "Release the lock before returning",
        "Fix double release of buffer",
        "Use fmt.Errorf to wrap errors",
        "Trim whitespace from user input",
        "Fix false positive in lint rule X",
        "Add a formatter",
        "Typography fixes",
        "Merge the branches",
        "Wipe the cache",
        "Releases page",
        "Revert \"Add the parser\"",
        "Add the parser",
        "fix: handle a release without notes",
        "feat(parser): add a lint pass",
    ];
    for subject in subjects {
        assert_eq!(card_of(subject, "")?.kind, ChangeKind::Normal, "{subject}");
    }
    Ok(())
}

/// A commit that only moved whitespace is trivial whatever its subject says.
#[test]
fn a_cosmetic_commit_is_trivial() -> TestResult {
    let context = CardContext {
        is_cosmetic: true,
        ..plain()
    };
    let card = read_card(&show("Add the parser", ""), &context)?;
    assert_eq!(card.kind, ChangeKind::Trivial);
    Ok(())
}

/// A commit over the limit, or one the miner listed no paths for, is a mass change; one at the
/// limit is not.
#[test]
fn a_commit_over_the_file_limit_is_a_mass_change() -> TestResult {
    let kind_for = |changed_paths| -> Result<ChangeKind, CardError> {
        let context = CardContext {
            changed_paths,
            max_files: 30,
            ..plain()
        };
        Ok(read_card(&show("Add the parser", ""), &context)?.kind)
    };
    assert_eq!(kind_for(ChangedPaths::Count(30))?, ChangeKind::Normal);
    assert_eq!(kind_for(ChangedPaths::Count(31))?, ChangeKind::Mass);
    assert_eq!(kind_for(ChangedPaths::OverMinerLimit)?, ChangeKind::Mass);
    Ok(())
}

/// The order of the kinds when several apply: mass over bot over trivial over normal.
#[test]
fn the_kinds_take_precedence_in_a_fixed_order() -> TestResult {
    let kind = |changed_paths, author: &str, subject: &str, is_cosmetic| {
        let context = CardContext {
            changed_paths,
            is_cosmetic,
            ..plain()
        };
        read_card(&show_by(author, "Cy", subject, ""), &context).map(|card| card.kind)
    };
    let many = ChangedPaths::Count(500);
    let few = ChangedPaths::Count(1);
    assert_eq!(kind(many, "x[bot]", "Bump y", true)?, ChangeKind::Mass);
    assert_eq!(kind(many, "Ann", "Bump y", false)?, ChangeKind::Mass);
    assert_eq!(kind(few, "x[bot]", "Bump y", true)?, ChangeKind::Bot);
    assert_eq!(kind(few, "Ann", "Bump y", false)?, ChangeKind::Trivial);
    assert_eq!(kind(few, "Ann", "Add y", true)?, ChangeKind::Trivial);
    assert_eq!(kind(few, "Ann", "Add y", false)?, ChangeKind::Normal);
    Ok(())
}
