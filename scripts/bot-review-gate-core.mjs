// Auto-post `review-gate` for Dependabot PRs (TASK-186) — the pure decision core.
//
// The bare `review-gate` status is posted only by a human `/ship` run. Dependabot
// never runs `/ship`, so its dependency PRs are permanently BLOCKED by the required
// `review-gate` check even when all real CI is green. This module decides whether a
// completed CI run belongs to a Dependabot PR whose real machine gates are all
// green — the ONLY case in which the caller may vouch `review-gate=success` on the
// PR head SHA. It never rubber-stamps: if any prerequisite gate is not green, or the
// run isn't a same-repo Dependabot PR, the decision is "don't post".
//
// Pure and side-effect-free so the guard logic is unit-tested headlessly; the I/O
// runner (`bot-review-gate.mjs`) does the gh calls.

// The contexts that must ALL be green before we vouch `review-gate` for a bot PR.
// These are the machine gates MERGE_POLICY.md requires on the default branch —
// everything in the required set EXCEPT `review-gate` itself. Keep this in sync
// with the branch-protection required checks (and MERGE_POLICY.md § Dependabot):
// vouching `review-gate` while one of these is missing would under-gate bot PRs.
export const PREREQUISITE_CONTEXTS = Object.freeze([
  "review-gate-tests",
  "review-gate-mistral",
  "review-gate-leak-scan",
]);

// The status context this workflow posts.
export const VOUCHED_CONTEXT = "review-gate";

// Only PRs authored by this actor, on a branch with this prefix, qualify.
export const BOT_ACTOR = "dependabot[bot]";
export const BOT_BRANCH_PREFIX = "dependabot/";

/**
 * Decide whether to post `review-gate=success` for a completed CI run.
 *
 * @param {object} input
 * @param {string} input.event      - workflow_run.event (want "pull_request")
 * @param {string} input.actor      - workflow_run.actor.login (want BOT_ACTOR)
 * @param {string} input.headBranch - workflow_run.head_branch (want BOT_BRANCH_PREFIX*)
 * @param {string} input.headRepo   - workflow_run.head_repository.full_name
 * @param {string} input.repo       - github.repository (base repo full name)
 * @param {Record<string,string>} input.statuses - context -> latest state
 *        (e.g. "success" | "failure" | "pending"); a missing context is treated
 *        as not-green.
 * @returns {{ post: boolean, reason: string }}
 */
export function decideBotReviewGate(input) {
  const { event, actor, headBranch, headRepo, repo, statuses = {} } = input || {};

  if (event !== "pull_request") {
    return { post: false, reason: `not a pull_request run (event=${event ?? "?"})` };
  }
  if (actor !== BOT_ACTOR) {
    return { post: false, reason: `actor ${actor ?? "?"} is not ${BOT_ACTOR}` };
  }
  // Same-repo guard: Dependabot branches live on the origin repo, not a fork.
  // A mismatch means we can't trust the head, so never vouch.
  if (!headRepo || headRepo !== repo) {
    return { post: false, reason: `head repo ${headRepo ?? "?"} is not ${repo ?? "?"}` };
  }
  if (typeof headBranch !== "string" || !headBranch.startsWith(BOT_BRANCH_PREFIX)) {
    return { post: false, reason: `branch ${headBranch ?? "?"} is not a ${BOT_BRANCH_PREFIX}* branch` };
  }

  const notGreen = PREREQUISITE_CONTEXTS.filter((c) => statuses[c] !== "success");
  if (notGreen.length > 0) {
    const detail = notGreen.map((c) => `${c}=${statuses[c] ?? "missing"}`).join(", ");
    return { post: false, reason: `prerequisite gates not green: ${detail}` };
  }

  return {
    post: true,
    reason: `all prerequisite gates green (${PREREQUISITE_CONTEXTS.join(", ")})`,
  };
}
