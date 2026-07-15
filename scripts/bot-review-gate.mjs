#!/usr/bin/env node
// Auto-post `review-gate` for Dependabot PRs (TASK-186) — the I/O runner the
// GitHub Action shells out to. Thin glue around the pure core
// (`bot-review-gate-core.mjs`): read the triggering `workflow_run` event, fetch
// the head commit's statuses, and — only if the core says so — post
// `review-gate=success` on that SHA.
//
// Runs from a `workflow_run` trigger, so it executes in the DEFAULT-branch context
// with the base repo's token (never PR-supplied code) — the safe pattern for a
// privileged reaction to a bot PR. It re-fires per rebase because each new head SHA
// re-runs the prerequisite gates.
//
// This runner is deliberately un-unit-tested (gh + FS glue, per repo doctrine); the
// decision logic it defers to is covered by bot-review-gate-core.test.mjs. No
// untrusted PR text is interpolated into a shell: gh is invoked via execFileSync
// argv vectors, and the PR branch/actor cross in as data only.
//
// ENV:
//   GITHUB_EVENT_PATH (required) — path to the workflow_run event JSON (set by Actions).
//   REPO / GITHUB_REPOSITORY     — "owner/name" of the base repo.
//   GH_TOKEN / GITHUB_TOKEN      — token with `statuses: write` (used by gh).

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { decideBotReviewGate, VOUCHED_CONTEXT } from "./bot-review-gate-core.mjs";

const repo = process.env.REPO || process.env.GITHUB_REPOSITORY;
const eventPath = process.env.GITHUB_EVENT_PATH;

function gh(args) {
  return execFileSync("gh", args, { encoding: "utf8" });
}

function main() {
  if (!repo || !eventPath) {
    console.log(`skip: missing REPO (${repo}) or GITHUB_EVENT_PATH (${eventPath})`);
    return;
  }

  const event = JSON.parse(readFileSync(eventPath, "utf8"));
  const run = event.workflow_run || {};
  const headSha = run.head_sha;

  // Fetch the latest commit status per context on the head SHA. The combined
  // status endpoint returns one entry per context in `.statuses` — our gates are
  // posted as commit statuses (not check-runs), so this is the right source.
  let statuses = {};
  if (headSha) {
    try {
      const combined = JSON.parse(gh(["api", `repos/${repo}/commits/${headSha}/status`]));
      for (const s of combined.statuses || []) statuses[s.context] = s.state;
    } catch (e) {
      console.log(`skip: could not read statuses for ${headSha}: ${e.message}`);
      return;
    }
  }

  const decision = decideBotReviewGate({
    event: run.event,
    actor: run.actor?.login,
    headBranch: run.head_branch,
    headRepo: run.head_repository?.full_name,
    repo,
    statuses,
  });

  if (!decision.post) {
    console.log(`no-op: ${decision.reason}`);
    return;
  }

  // Vouch review-gate on the head SHA. Gated entirely by the core above, so this
  // only runs for a same-repo Dependabot PR with every machine gate already green.
  gh([
    "api",
    `repos/${repo}/statuses/${headSha}`,
    "-f",
    "state=success",
    "-f",
    `context=${VOUCHED_CONTEXT}`,
    "-f",
    "description=Dependabot PR: machine gates green (tests + mistral + leak-scan)",
    "-f",
    `target_url=${process.env.GITHUB_SERVER_URL || "https://github.com"}/${repo}/actions/runs/${process.env.GITHUB_RUN_ID || ""}`,
  ]);
  console.log(`posted ${VOUCHED_CONTEXT}=success on ${headSha}: ${decision.reason}`);
}

main();
