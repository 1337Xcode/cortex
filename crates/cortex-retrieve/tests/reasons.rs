//! Why a file is listed: the strongest true reason chosen, written in at most 60 characters.

use cortex_retrieve::{ItemEvidence, MAX_REASON_CHARS, Reason, reason_of};

/// Every reason written as typed here.
#[test]
fn each_reason_is_written_as_typed_here() {
    let cases = [
        (Reason::Named, "named in the prompt"),
        (Reason::EditedHere, "edited earlier in this session"),
        (
            Reason::Matches(vec![
                "order".into(),
                "create".into(),
                "total".into(),
                "tax".into(),
            ]),
            "matches order, create, total",
        ),
        (
            Reason::ChangesWith {
                other: "service.ts".into(),
                together: 9,
                of: 12,
            },
            "changes with service.ts in 9 of 12 commits",
        ),
        (Reason::RefersTo("money.ts".into()), "refers to money.ts"),
        (
            Reason::ReferredToBy("route.ts".into()),
            "referred to by route.ts",
        ),
        (Reason::LikeEarlierWork, "changed in similar earlier tasks"),
        (
            Reason::WorkedOnWith("schema.prisma".into()),
            "worked on with schema.prisma before",
        ),
        (Reason::ReadHere, "read earlier in this session"),
    ];
    for (reason, expected) in cases {
        assert_eq!(reason.text(), expected);
    }
}

/// A name too long for 60 characters is cut, its end shown as `...`: "changes with " is 13
/// characters and " in 9 of 12 commits" 19, so a name gets 28, the first 25 of it and `...`.
#[test]
fn a_long_name_is_cut_to_fit() {
    let long = "a_very_long_file_name_that_goes_on_and_on.py";
    let reason = Reason::ChangesWith {
        other: long.into(),
        together: 9,
        of: 12,
    };
    let text = reason.text();
    assert_eq!(
        text,
        "changes with a_very_long_file_name_tha... in 9 of 12 commits"
    );
    assert_eq!(text.chars().count(), MAX_REASON_CHARS);
    for reason in [
        Reason::RefersTo(long.repeat(3)),
        Reason::ReferredToBy(long.repeat(3)),
        Reason::WorkedOnWith(long.repeat(3)),
        Reason::Matches(vec![long.repeat(3)]),
    ] {
        assert!(
            reason.text().chars().count() <= MAX_REASON_CHARS,
            "{reason:?}"
        );
    }
}

/// The strongest reason the evidence holds wins, in the order the reasons are listed; with none,
/// the file is not listed.
#[test]
fn the_strongest_reason_is_chosen() {
    let all = ItemEvidence {
        is_named: true,
        is_edited_here: true,
        words: vec!["order".into()],
        changes_with: Some(("a.py".into(), 2, 3)),
        refers_to: Some("b.py".into()),
        referred_to_by: Some("c.py".into()),
        is_like_earlier_work: true,
        worked_on_with: Some("d.py".into()),
        is_read_here: true,
    };
    let mut evidence = all.clone();
    let mut chosen = Vec::new();
    while let Some(reason) = reason_of(&evidence) {
        chosen.push(reason.text());
        match reason {
            Reason::Named => evidence.is_named = false,
            Reason::EditedHere => evidence.is_edited_here = false,
            Reason::Matches(_) => evidence.words.clear(),
            Reason::ChangesWith { .. } => evidence.changes_with = None,
            Reason::RefersTo(_) => evidence.refers_to = None,
            Reason::ReferredToBy(_) => evidence.referred_to_by = None,
            Reason::LikeEarlierWork => evidence.is_like_earlier_work = false,
            Reason::WorkedOnWith(_) => evidence.worked_on_with = None,
            Reason::ReadHere => evidence.is_read_here = false,
        }
    }
    assert_eq!(
        chosen,
        [
            "named in the prompt",
            "edited earlier in this session",
            "matches order",
            "changes with a.py in 2 of 3 commits",
            "refers to b.py",
            "referred to by c.py",
            "changed in similar earlier tasks",
            "worked on with d.py before",
            "read earlier in this session",
        ]
    );
    assert_eq!(reason_of(&ItemEvidence::default()), None);
}
