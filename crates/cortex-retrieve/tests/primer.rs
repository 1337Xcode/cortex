//! The session primer: the repository's mode and size, its landmarks, and what Cortex adds, the
//! same bytes from one session to the next.

mod common;

use common::{Repo, TestResult};
use cortex_core::{FileFlags, InterestTier, ScaleMode};
use cortex_retrieve::{GraphSettings, MAX_PRIMER_LANDMARKS, primer, read_graph};

/// `lines` lines of text.
fn text(lines: usize) -> String {
    "x = 1\n".repeat(lines)
}

/// The primer of `repo`'s store, read twice to show it is the same bytes each time.
fn primer_of(repo: &Repo) -> TestResult<Option<String>> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let first = primer(reader.queries(), &graph)?;
    assert_eq!(first, primer(reader.queries(), &graph)?);
    Ok(first)
}

/// Worked by hand: three Python files of 10, 5 and 2 lines count, the generated one does not, so
/// the repository holds about 3 source files and 17 lines; `src/b.py` and `a.py` are landmarks,
/// listed by path (`src/b.py` is stored first and is the more interesting, so neither the store's
/// order nor interest puts `a.py` first), and `c.py`, merely interesting, is not.
#[test]
fn the_primer_names_the_mode_size_and_landmarks() -> TestResult {
    let mut repo = Repo::new()?;
    let b = repo.file("src/b.py", &text(5), &[], FileFlags::empty())?[0];
    let a = repo.file("a.py", &text(10), &[], FileFlags::empty())?[0];
    let c = repo.file("c.py", &text(2), &[], FileFlags::empty())?[0];
    repo.file("gen.py", &text(100), &[], FileFlags::GENERATED)?;
    repo.interest(&[
        (a, (4.0, 0), InterestTier::Landmark),
        (b, (6.0, 0), InterestTier::Landmark),
        (c, (1.0, 0), InterestTier::Interesting),
    ])?;
    repo.mode(ScaleMode::Medium)?;
    let expected = "\
[cortex] Project map · medium mode · about 3 source files, 17 lines
Landmarks: a.py, src/b.py
Cortex adds the code a prompt points at before the agent starts, when the evidence agrees.
";
    assert_eq!(primer_of(&repo)?.as_deref(), Some(expected));
    Ok(())
}

/// In tiny mode Cortex adds nothing to prompts, and says so; with no landmark there is no
/// landmark line.
#[test]
fn a_tiny_repository_is_told_nothing_is_added() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", &text(3), &[], FileFlags::empty())?;
    repo.mode(ScaleMode::Tiny)?;
    let expected = "\
[cortex] Project map · tiny mode · about 1 source file, 3 lines
Cortex records what agents do here; with this little code it adds nothing to prompts.
";
    assert_eq!(primer_of(&repo)?.as_deref(), Some(expected));
    Ok(())
}

/// Before the first index has set the mode there is no primer.
#[test]
fn no_mode_no_primer() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", &text(3), &[], FileFlags::empty())?;
    assert_eq!(primer_of(&repo)?, None);
    Ok(())
}

/// Of ten landmarks, the eight most interesting are named, by path: `f0.py`, scored 1, and
/// `f1.py`, scored 2, are left out.
#[test]
fn at_most_eight_landmarks_are_named() -> TestResult {
    let mut repo = Repo::new()?;
    let mut rows = Vec::new();
    for at in 0..10_u32 {
        let node = repo.file(&format!("f{at}.py"), &text(1), &[], FileFlags::empty())?[0];
        rows.push((node, (f64::from(at + 1), 0), InterestTier::Landmark));
    }
    repo.interest(&rows)?;
    repo.mode(ScaleMode::Small)?;
    let primer = primer_of(&repo)?.ok_or("a primer")?;
    let named: Vec<String> = (2..10).map(|at| format!("f{at}.py")).collect();
    assert_eq!(named.len(), MAX_PRIMER_LANDMARKS);
    assert!(
        primer.contains(&format!("Landmarks: {}\n", named.join(", "))),
        "{primer}"
    );
    Ok(())
}
