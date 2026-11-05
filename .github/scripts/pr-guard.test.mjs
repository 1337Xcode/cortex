// Tests of the pull request guard. Run with `node --test .github/scripts/*.test.mjs`.
// Every expected label set is worked out by hand from the case's dates and the thresholds:
// 30 days for a new account, 24 hours for a fresh fork, both only for a first pull request here.

import { readFileSync } from "node:fs";
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  COMMENT_MARKER,
  clearedComment,
  decide,
  hasProof,
  parseVouched,
  run,
} from "./pr-guard.mjs";

/** The repository's own pull request template, as a new pull request's body starts. */
const TEMPLATE_BODY = readFileSync(new URL("../pull_request_template.md", import.meta.url), "utf8");

const EMPTY_FENCE = "```text\n\n```";

/** The template with its Verification section filled in. */
const PROVEN_BODY = TEMPLATE_BODY.replace(EMPTY_FENCE, "```text\nxtask: all checks passed\n```");

/** A first-time contributor's pull request opened 2026-11-05 12:00 UTC from a fork. */
function facts(overrides = {}) {
  return {
    login: "someone",
    isBot: false,
    association: "FIRST_TIME_CONTRIBUTOR",
    accountCreatedAt: "2020-01-01T00:00:00Z",
    openedAt: "2026-11-05T12:00:00Z",
    forkCreatedAt: "2026-01-01T00:00:00Z",
    mergedBefore: 0,
    body: PROVEN_BODY,
    labels: [],
    ...overrides,
  };
}

const NOBODY = new Set();

test("the template's empty fence is where the proof goes", () => {
  assert.equal(TEMPLATE_BODY.split(EMPTY_FENCE).length, 2);
});

test("an old account, an old fork and a filled Verification get nothing", () => {
  assert.deepEqual(decide(facts(), NOBODY), { skipped: null, add: [], remove: [], comment: null });
});

test("a ten-day-old account, a two-hour-old fork and no proof get all three labels", () => {
  const decision = decide(
    facts({
      accountCreatedAt: "2026-10-26T12:00:00Z",
      forkCreatedAt: "2026-11-05T10:00:00Z",
      body: TEMPLATE_BODY,
    }),
    NOBODY,
  );
  assert.equal(decision.skipped, null);
  assert.deepEqual(decision.add, ["new-account", "fresh-fork", "needs-proof"]);
  assert.deepEqual(decision.remove, []);
  assert.ok(decision.comment.startsWith(COMMENT_MARKER));
  for (const label of ["new-account", "fresh-fork", "needs-proof"]) {
    assert.ok(decision.comment.includes(`\`${label}\``), label);
  }
  assert.ok(decision.comment.includes("nothing is blocked and nothing will be closed"));
});

test("the account age is strict: one second under 30 days is new, exactly 30 days is not", () => {
  // 2026-11-05T12:00:00Z minus 30 days is 2026-10-06T12:00:00Z.
  assert.deepEqual(decide(facts({ accountCreatedAt: "2026-10-06T12:00:01Z" }), NOBODY).add, [
    "new-account",
  ]);
  assert.deepEqual(decide(facts({ accountCreatedAt: "2026-10-06T12:00:00Z" }), NOBODY).add, []);
});

test("the fork age is strict: 23 hours is fresh, 25 hours is not, no fork is never fresh", () => {
  assert.deepEqual(decide(facts({ forkCreatedAt: "2026-11-04T13:00:00Z" }), NOBODY).add, [
    "fresh-fork",
  ]);
  assert.deepEqual(decide(facts({ forkCreatedAt: "2026-11-04T11:00:00Z" }), NOBODY).add, []);
  assert.deepEqual(decide(facts({ forkCreatedAt: null }), NOBODY).add, []);
});

test("a returning contributor is never new or fresh, but still needs proof", () => {
  const decision = decide(
    facts({
      mergedBefore: 1,
      accountCreatedAt: "2026-11-01T00:00:00Z",
      forkCreatedAt: "2026-11-05T11:00:00Z",
      body: TEMPLATE_BODY,
    }),
    NOBODY,
  );
  assert.deepEqual(decision.add, ["needs-proof"]);
});

test("labels already there are not added again, and one no longer wanted is removed", () => {
  const still = decide(facts({ body: "", labels: ["needs-proof", "bug"] }), NOBODY);
  assert.deepEqual([still.add, still.remove], [[], []]);
  assert.notEqual(still.comment, null);
  const proven = decide(facts({ labels: ["needs-proof", "bug"] }), NOBODY);
  assert.deepEqual([proven.add, proven.remove, proven.comment], [[], ["needs-proof"], null]);
});

test("bots, maintainers, the vouched and maintainer-ok are skipped and their guard labels cleared", () => {
  const labels = ["new-account", "needs-proof", "bug"];
  const young = { accountCreatedAt: "2026-11-01T00:00:00Z", body: "", labels };
  const cases = [
    [facts({ ...young, isBot: true }), NOBODY, "the author is a bot"],
    [facts({ ...young, association: "OWNER" }), NOBODY, "the author is a maintainer or collaborator"],
    [facts({ ...young, association: "COLLABORATOR" }), NOBODY, "the author is a maintainer or collaborator"],
    [facts({ ...young, login: "SomeOne" }), new Set(["someone"]), "the author is in .github/VOUCHED"],
    [facts({ ...young, labels: [...labels, "maintainer-ok"] }), NOBODY, "a maintainer added maintainer-ok"],
  ];
  for (const [input, vouched, reason] of cases) {
    assert.deepEqual(decide(input, vouched), {
      skipped: reason,
      add: [],
      remove: ["new-account", "needs-proof"],
      comment: null,
    });
  }
});

test("a contributor's association does not skip the guard", () => {
  const decision = decide(facts({ association: "CONTRIBUTOR", body: "" }), NOBODY);
  assert.deepEqual(decision.add, ["needs-proof"]);
});

test("proof is anything in the Verification section beyond comments and an empty code block", () => {
  assert.equal(hasProof(TEMPLATE_BODY), false);
  assert.equal(hasProof(PROVEN_BODY), true);
  assert.equal(hasProof("## Verification\nRan `cortex status` on my repo; it printed index: ready."), true);
  assert.equal(hasProof("### Verification\nran it"), true);
  assert.equal(hasProof("## Verification (Linux)\nran it"), true);
  assert.equal(hasProof("## Verification\n### On Linux\nran it\n## Checklist"), true);
  assert.equal(hasProof("## verification   \r\n\r\n~~~\r\n~~~\r\n"), false);
  assert.equal(hasProof("## Verifications are hard\nno"), false);
  assert.equal(hasProof("# Verification\nran it"), false);
  assert.equal(hasProof("## Problem\nx\n## Change\ny"), false);
  assert.equal(hasProof("## Verification\n## Checklist\n- [x] tested"), false);
  assert.equal(hasProof(""), false);
  assert.equal(hasProof(null), false);
  assert.equal(hasProof(undefined), false);
});

test("the vouch file is one login per line, with comments, blank lines and any case", () => {
  const vouched = parseVouched("# trusted contributors\nAlice\n\n  bob  # since 2026\r\n#carol\n");
  assert.deepEqual([...vouched].sort(), ["alice", "bob"]);
});

test("the cleared comment keeps the marker, so the same comment is edited again later", () => {
  assert.ok(clearedComment().startsWith(`${COMMENT_MARKER}\n`));
});

/**
 * An API client that answers from `world` and records every call by name. A write it is not
 * expected to make still records, so a test can assert that none happened.
 */
function fakeGithub(world) {
  const calls = [];
  const call = (name, answer) => async (args) => {
    calls.push([name, args]);
    return answer(args);
  };
  const github = {
    rest: {
      pulls: { get: call("pulls.get", () => ({ data: world.pr })) },
      users: { getByUsername: call("users.getByUsername", () => ({ data: world.account })) },
      search: {
        issuesAndPullRequests: call("search", () => ({ data: { total_count: world.merged } })),
      },
      issues: {
        addLabels: call("addLabels", () => ({})),
        removeLabel: call("removeLabel", (args) => {
          if (world.missingLabels?.includes(args.name)) {
            throw Object.assign(new Error("Label does not exist"), { status: 404 });
          }
          return {};
        }),
        listComments: Symbol("listComments"),
        createComment: call("createComment", () => ({})),
        updateComment: call("updateComment", () => ({})),
      },
    },
    paginate: async (method, args) => {
      assert.equal(method, github.rest.issues.listComments);
      calls.push(["listComments", args]);
      return world.comments ?? [];
    },
  };
  return { github, calls };
}

/** A pull request from a ten-day-old account's two-hour-old fork, its template unfilled. */
function newcomerWorld(overrides = {}) {
  return {
    pr: {
      user: { login: "someone", type: "User" },
      author_association: "FIRST_TIME_CONTRIBUTOR",
      labels: [],
      created_at: "2026-11-05T12:00:00Z",
      body: TEMPLATE_BODY,
      head: { repo: { full_name: "someone/cortex", created_at: "2026-11-05T10:00:00Z" } },
    },
    account: { created_at: "2026-10-26T12:00:00Z" },
    merged: 0,
    ...overrides,
  };
}

const TARGET = { owner: "owner", repo: "cortex", number: 7, vouched: NOBODY };

test("run labels a newcomer's pull request and posts one comment", async () => {
  const { github, calls } = fakeGithub(newcomerWorld());
  const { login, decision } = await run({ github, ...TARGET, dryRun: false });
  assert.equal(login, "someone");
  assert.deepEqual(decision.add, ["new-account", "fresh-fork", "needs-proof"]);
  assert.deepEqual(
    calls.map(([name]) => name),
    ["pulls.get", "users.getByUsername", "search", "addLabels", "listComments", "createComment"],
  );
  assert.deepEqual(calls[1][1], { username: "someone" });
  assert.deepEqual(calls[2][1], {
    q: "repo:owner/cortex is:pr is:merged author:someone",
    per_page: 1,
  });
  assert.deepEqual(calls[3][1], {
    owner: "owner",
    repo: "cortex",
    issue_number: 7,
    labels: ["new-account", "fresh-fork", "needs-proof"],
  });
  assert.equal(calls[5][1].issue_number, 7);
  assert.equal(calls[5][1].body, decision.comment);
});

test("a dry run reads and decides but writes nothing", async () => {
  const { github, calls } = fakeGithub(newcomerWorld());
  const { decision } = await run({ github, ...TARGET, dryRun: true });
  assert.deepEqual(decision.add, ["new-account", "fresh-fork", "needs-proof"]);
  assert.deepEqual(
    calls.map(([name]) => name),
    ["pulls.get", "users.getByUsername", "search"],
  );
});

test("a maintainer's pull request costs no further call and only loses the guard's labels", async () => {
  const world = newcomerWorld();
  world.pr.author_association = "OWNER";
  world.pr.labels = [{ name: "needs-proof" }, { name: "bug" }];
  const { github, calls } = fakeGithub(world);
  const { decision } = await run({ github, ...TARGET, dryRun: false });
  assert.equal(decision.skipped, "the author is a maintainer or collaborator");
  assert.deepEqual(
    calls.map(([name]) => name),
    ["pulls.get", "removeLabel", "listComments"],
  );
  assert.equal(calls[1][1].name, "needs-proof");
});

test("the guard's earlier comment is edited, never another account's", async () => {
  const world = newcomerWorld({
    comments: [
      { id: 1, user: { login: "someone" }, body: `${COMMENT_MARKER}\nI wrote this` },
      { id: 2, user: { login: "github-actions[bot]" }, body: `${COMMENT_MARKER}\nold text` },
    ],
  });
  const { github, calls } = fakeGithub(world);
  await run({ github, ...TARGET, dryRun: false });
  const edits = calls.filter(([name]) => name === "updateComment");
  assert.equal(edits.length, 1);
  assert.equal(edits[0][1].comment_id, 2);
  assert.ok(!calls.some(([name]) => name === "createComment"));
});

test("an unchanged comment is not written again, and a cleared one says so", async () => {
  const proven = newcomerWorld();
  proven.pr.body = PROVEN_BODY;
  proven.account.created_at = "2020-01-01T00:00:00Z";
  proven.pr.head.repo.created_at = "2020-01-01T00:00:00Z";
  proven.pr.labels = [{ name: "needs-proof" }];
  proven.comments = [{ id: 2, user: { login: "github-actions[bot]" }, body: `${COMMENT_MARKER}\nold` }];
  const cleared = fakeGithub(proven);
  await run({ github: cleared.github, ...TARGET, dryRun: false });
  const edit = cleared.calls.find(([name]) => name === "updateComment");
  assert.equal(edit[1].body, clearedComment());

  proven.comments = [{ id: 2, user: { login: "github-actions[bot]" }, body: clearedComment() }];
  proven.pr.labels = [];
  const again = fakeGithub(proven);
  await run({ github: again.github, ...TARGET, dryRun: false });
  assert.ok(!again.calls.some(([name]) => name === "updateComment" || name === "createComment"));
});

test("a deleted fork is not fresh, and a label already gone is no error", async () => {
  const world = newcomerWorld({ missingLabels: ["fresh-fork"] });
  world.pr.head.repo = null;
  world.pr.labels = [{ name: "fresh-fork" }];
  const { github } = fakeGithub(world);
  const { decision } = await run({ github, ...TARGET, dryRun: false });
  assert.deepEqual(decision.add, ["new-account", "needs-proof"]);
  assert.deepEqual(decision.remove, ["fresh-fork"]);
});
