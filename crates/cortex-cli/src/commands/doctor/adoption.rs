//! The adoption check: whether Cortex sets repositories up on its own for this user, how many
//! folders it refused by its rules and the newest of them; and a failure when it refused this
//! repository, which hooks then leave alone until it is set up by hand.

use cortex_core::UserFolder;

use super::{Context, Finding};
use crate::adoption;

/// What adoption is for the user, a line each, and a failure when it refused the repository.
pub(super) fn state(context: &Context) -> Vec<Finding> {
    let Some(user) = UserFolder::of_current_user() else {
        return vec![Finding::Fail {
            what: "no home folder to find adoption's state in".to_owned(),
            fix: "set the home folder (HOME, or USERPROFILE on Windows)".to_owned(),
        }];
    };
    let read = adoption::lines(&user)
        .and_then(|lines| Ok((lines, adoption::refusal_of(&user, &context.root)?)));
    match read {
        Ok((lines, refused)) => {
            // The check's name says adoption already.
            let mut findings: Vec<Finding> = lines
                .iter()
                .map(|line| {
                    Finding::Pass(line.strip_prefix("adoption: ").unwrap_or(line).to_owned())
                })
                .collect();
            if let Some(refused) = refused {
                findings.push(Finding::Fail {
                    what: format!(
                        "Cortex refused to set this repository up on its own: {}",
                        refused.why
                    ),
                    fix: "run `cortex init` here to set it up by hand, or `cortex on` to forget \
                          the refusal once what refused it changed"
                        .to_owned(),
                });
            }
            findings
        }
        Err(error) => vec![Finding::Fail {
            what: format!("adoption's refusals cannot be read: {error:#}"),
            fix: "look at ~/.cortex/run/refused/".to_owned(),
        }],
    }
}
