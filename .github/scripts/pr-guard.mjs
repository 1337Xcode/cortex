// The pull request guard: which labels a pull request gets, what its one comment says, and the
// calls that read the facts and apply the decision. `decide` is a pure function of the facts, so
// the same pull request always gets the same answer; `run` reads the facts through the API client
// actions/github-script provides and applies the decision unless asked for a dry run.
//
// The guard only labels and explains. It never closes, never requests changes and never runs the
// pull request's code. A maintainer overrides it with the `maintainer-ok` label or by adding the
// author to `.github/VOUCHED`.

/** An account younger than this, when the pull request was opened, is labelled `new-account`. */
export const NEW_ACCOUNT_DAYS = 30;

/** A pull request opened this soon after its fork was made is labelled `fresh-fork`. */
export const FRESH_FORK_HOURS = 24;

/** The labels the guard manages. Any other label on the pull request is left alone. */
export const GUARD_LABELS = ["new-account", "fresh-fork", "needs-proof"];

/** The label a maintainer adds to take a pull request out of the guard's hands. */
export const OVERRIDE_LABEL = "maintainer-ok";

/** Marks the guard's own comment, so a later run edits it instead of posting another. */
export const COMMENT_MARKER = "<!-- cortex-pr-guard -->";

/** The account the workflow's token comments as; only its comments are ever edited. */
const GUARD_ACCOUNT = "github-actions[bot]";

/** How the repository knows the author: these associations are never labelled. */
const TRUSTED_ASSOCIATIONS = new Set(["OWNER", "MEMBER", "COLLABORATOR"]);

const MS_PER_HOUR = 60 * 60 * 1000;
const MS_PER_DAY = 24 * MS_PER_HOUR;

/**
 * The logins in a VOUCHED file: one per line, `#` starts a comment, case ignored.
 *
 * @param {string} text the file's contents
 * @returns {Set<string>} the logins, lower case
 */
export function parseVouched(text) {
  const logins = new Set();
  for (const line of text.split(/\r?\n/)) {
    const login = line.replace(/#.*/, "").trim().toLowerCase();
    if (login !== "") {
      logins.add(login);
    }
  }
  return logins;
}

/**
 * Whether the body's Verification section holds anything beyond the template's comments and an
 * empty code block. The section is a level 2 or 3 heading that starts with "Verification" and
 * runs to the next heading of its level or above. A body without it has no proof.
 *
 * @param {string | null | undefined} body the pull request's description
 * @returns {boolean}
 */
export function hasProof(body) {
  if (typeof body !== "string") {
    return false;
  }
  const lines = body.replace(/<!--[\s\S]*?-->/g, "").split(/\r?\n/);
  const heading = /^(#{2,3})\s+verification\b/i;
  const start = lines.findIndex((line) => heading.test(line.trim()));
  if (start === -1) {
    return false;
  }
  const level = heading.exec(lines[start].trim())[1].length;
  const ends = new RegExp(`^#{1,${level}}\\s`);
  const section = [];
  for (const line of lines.slice(start + 1)) {
    if (ends.test(line.trim())) {
      break;
    }
    section.push(line);
  }
  return section.some((line) => {
    const text = line.trim();
    return text !== "" && !/^(```|~~~)\w*$/.test(text);
  });
}

/**
 * Why the guard leaves an author's pull request alone, or `null` when it judges it. Needs nothing
 * the pull request itself does not carry, so a skipped author costs no further API call.
 *
 * @param {object} author who opened it
 * @param {string} author.login the author's login
 * @param {boolean} author.isBot whether the author is a bot account
 * @param {string} author.association the pull request's `author_association`
 * @param {Set<string>} vouched the logins in `.github/VOUCHED`, lower case
 * @param {Set<string>} labels the labels on the pull request now
 * @returns {string | null}
 */
export function skipReason(author, vouched, labels) {
  if (author.isBot) {
    return "the author is a bot";
  }
  if (TRUSTED_ASSOCIATIONS.has(author.association)) {
    return "the author is a maintainer or collaborator";
  }
  if (vouched.has(author.login.toLowerCase())) {
    return "the author is in .github/VOUCHED";
  }
  if (labels.has(OVERRIDE_LABEL)) {
    return `a maintainer added ${OVERRIDE_LABEL}`;
  }
  return null;
}

/**
 * What the guard does with one pull request.
 *
 * @param {object} facts what was read from GitHub
 * @param {string} facts.login the author's login
 * @param {boolean} facts.isBot whether the author is a bot account
 * @param {string} facts.association the pull request's `author_association`
 * @param {string} facts.accountCreatedAt when the author's account was made (ISO 8601)
 * @param {string} facts.openedAt when the pull request was opened (ISO 8601)
 * @param {string | null} facts.forkCreatedAt when the head repository was made, if it is a fork
 * @param {number} facts.mergedBefore the author's pull requests already merged in this repository
 * @param {string | null} facts.body the pull request's description
 * @param {string[]} facts.labels the labels on the pull request now
 * @param {Set<string>} vouched the logins in `.github/VOUCHED`, lower case
 * @returns {{ skipped: string | null, add: string[], remove: string[], comment: string | null }}
 */
export function decide(facts, vouched) {
  const present = new Set(facts.labels);
  const skip = skipReason(facts, vouched, present);
  if (skip !== null) {
    return skippedDecision(skip, present);
  }
  const wanted = [];
  const opened = Date.parse(facts.openedAt);
  const isNewcomer = facts.mergedBefore === 0;
  if (isNewcomer && opened - Date.parse(facts.accountCreatedAt) < NEW_ACCOUNT_DAYS * MS_PER_DAY) {
    wanted.push("new-account");
  }
  if (
    isNewcomer &&
    facts.forkCreatedAt !== null &&
    opened - Date.parse(facts.forkCreatedAt) < FRESH_FORK_HOURS * MS_PER_HOUR
  ) {
    wanted.push("fresh-fork");
  }
  if (!hasProof(facts.body)) {
    wanted.push("needs-proof");
  }
  const managed = GUARD_LABELS.filter((label) => present.has(label));
  return {
    skipped: null,
    add: wanted.filter((label) => !present.has(label)),
    remove: managed.filter((label) => !wanted.includes(label)),
    comment: wanted.length === 0 ? null : commentFor(wanted),
  };
}

/**
 * The decision for a pull request the guard leaves alone: its own labels come off, nothing else.
 *
 * @param {string} reason why it is left alone
 * @param {Set<string>} present the labels on the pull request now
 * @returns {{ skipped: string, add: string[], remove: string[], comment: null }}
 */
function skippedDecision(reason, present) {
  const managed = GUARD_LABELS.filter((label) => present.has(label));
  return { skipped: reason, add: [], remove: managed, comment: null };
}

/** What each label tells the author, in the order the labels are decided. */
const EXPLANATIONS = {
  "new-account":
    "Your account is less than " +
    `${NEW_ACCOUNT_DAYS} days old and this is your first pull request here, so a maintainer ` +
    "will read it with extra care before CI's results count.",
  "fresh-fork":
    `The pull request came within ${FRESH_FORK_HOURS} hours of the fork. That is fine; it ` +
    "only means a maintainer will check the change was run, not just written.",
  "needs-proof":
    "The Verification section is empty. Please paste the last lines of `cargo xtask verify` " +
    "and say what you ran by hand and what it printed.",
};

/**
 * The guard's one comment for the labels it wants.
 *
 * @param {string[]} labels the labels decided, in order
 * @returns {string}
 */
function commentFor(labels) {
  const lines = [
    COMMENT_MARKER,
    "Thanks for the pull request. An automatic check added a label or two; nothing is blocked " +
      "and nothing will be closed by it.",
    "",
    ...labels.map((label) => `- \`${label}\`: ${EXPLANATIONS[label]}`),
    "",
    "A maintainer can clear these at any time. CONTRIBUTING.md explains what a pull request " +
      "needs and how a regular contributor is added to `.github/VOUCHED`.",
  ];
  return lines.join("\n");
}

/**
 * What the guard's earlier comment becomes once none of its labels applies any more.
 *
 * @returns {string}
 */
export function clearedComment() {
  return [
    COMMENT_MARKER,
    "Thanks. The labels the automatic check added earlier no longer apply and have been removed.",
  ].join("\n");
}

/**
 * Judges pull request `number` and, unless `dryRun`, applies the decision: labels added and
 * removed, and the one comment posted or edited. Reads only through `github`, the API client
 * actions/github-script provides; never the pull request's files.
 *
 * @param {object} options
 * @param {object} options.github the API client (`rest.*` and `paginate`)
 * @param {string} options.owner the repository's owner
 * @param {string} options.repo the repository's name
 * @param {number} options.number the pull request's number
 * @param {Set<string>} options.vouched the logins in `.github/VOUCHED`, lower case
 * @param {boolean} options.dryRun decide and report only
 * @returns {Promise<{ login: string, decision: object }>} who opened it and what was decided
 */
export async function run({ github, owner, repo, number, vouched, dryRun }) {
  const { data: pr } = await github.rest.pulls.get({ owner, repo, pull_number: number });
  const author = {
    login: pr.user.login,
    isBot: pr.user.type === "Bot",
    association: pr.author_association,
  };
  const labels = pr.labels.map((label) => label.name);
  const skip = skipReason(author, vouched, new Set(labels));
  const decision =
    skip !== null
      ? skippedDecision(skip, new Set(labels))
      : decide(await factsOf(github, owner, repo, pr, { ...author, labels }), vouched);
  if (!dryRun) {
    await apply(github, owner, repo, number, decision);
  }
  return { login: author.login, decision };
}

/**
 * The facts `decide` needs that the pull request does not carry: the account's age and the
 * author's merged pull requests here.
 *
 * @returns {Promise<object>}
 */
async function factsOf(github, owner, repo, pr, known) {
  const { data: account } = await github.rest.users.getByUsername({ username: known.login });
  const { data: merged } = await github.rest.search.issuesAndPullRequests({
    q: `repo:${owner}/${repo} is:pr is:merged author:${known.login}`,
    per_page: 1,
  });
  const head = pr.head.repo;
  const isFork = head !== null && head.full_name !== `${owner}/${repo}`;
  return {
    ...known,
    accountCreatedAt: account.created_at,
    openedAt: pr.created_at,
    forkCreatedAt: isFork ? head.created_at : null,
    mergedBefore: merged.total_count,
    body: pr.body,
  };
}

/**
 * Applies `decision` to pull request `number`: its labels, then the guard's one comment, edited
 * in place when the guard already wrote one, and cleared when its labels no longer apply.
 *
 * @returns {Promise<void>}
 */
async function apply(github, owner, repo, number, decision) {
  if (decision.add.length > 0) {
    await github.rest.issues.addLabels({ owner, repo, issue_number: number, labels: decision.add });
  }
  for (const name of decision.remove) {
    try {
      await github.rest.issues.removeLabel({ owner, repo, issue_number: number, name });
    } catch (error) {
      // Another run, or a maintainer, removed it first.
      if (error.status !== 404) {
        throw error;
      }
    }
  }
  const comments = await github.paginate(github.rest.issues.listComments, {
    owner,
    repo,
    issue_number: number,
    per_page: 100,
  });
  const earlier = comments.find(
    (comment) => comment.user.login === GUARD_ACCOUNT && comment.body.startsWith(COMMENT_MARKER),
  );
  const body = decision.comment ?? (earlier ? clearedComment() : null);
  if (body === null || (earlier && earlier.body === body)) {
    return;
  }
  if (earlier) {
    await github.rest.issues.updateComment({ owner, repo, comment_id: earlier.id, body });
  } else {
    await github.rest.issues.createComment({ owner, repo, issue_number: number, body });
  }
}
