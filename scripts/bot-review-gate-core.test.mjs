import { describe, it, expect } from "vitest";
import {
  decideBotReviewGate,
  PREREQUISITE_CONTEXTS,
  BOT_ACTOR,
} from "./bot-review-gate-core.mjs";

// A same-repo Dependabot PR run with every prerequisite gate green.
// (Placeholder repo name — the logic never depends on the real slug.)
const REPO = "example-org/example-repo";
const greenStatuses = () =>
  Object.fromEntries(PREREQUISITE_CONTEXTS.map((c) => [c, "success"]));
const baseInput = (over = {}) => ({
  event: "pull_request",
  actor: BOT_ACTOR,
  headBranch: "dependabot/cargo/quick-xml-0.41.0",
  headRepo: REPO,
  repo: REPO,
  statuses: greenStatuses(),
  ...over,
});

describe("decideBotReviewGate", () => {
  it("posts when a same-repo Dependabot PR has all prerequisite gates green", () => {
    const r = decideBotReviewGate(baseInput());
    expect(r.post).toBe(true);
  });

  it("does NOT post when a prerequisite gate is failing", () => {
    const statuses = { ...greenStatuses(), "review-gate-mistral": "failure" };
    const r = decideBotReviewGate(baseInput({ statuses }));
    expect(r.post).toBe(false);
    expect(r.reason).toContain("review-gate-mistral=failure");
  });

  it("does NOT post when a prerequisite gate is still pending", () => {
    const statuses = { ...greenStatuses(), "review-gate-tests": "pending" };
    expect(decideBotReviewGate(baseInput({ statuses })).post).toBe(false);
  });

  it("does NOT post when a prerequisite gate is missing entirely", () => {
    const statuses = { ...greenStatuses() };
    delete statuses["review-gate-leak-scan"];
    const r = decideBotReviewGate(baseInput({ statuses }));
    expect(r.post).toBe(false);
    expect(r.reason).toContain("review-gate-leak-scan=missing");
  });

  it("does NOT post for a non-Dependabot actor even if all gates are green", () => {
    expect(decideBotReviewGate(baseInput({ actor: "some-human" })).post).toBe(false);
  });

  it("does NOT post for a fork (head repo != base repo)", () => {
    const r = decideBotReviewGate(baseInput({ headRepo: "attacker/example-repo" }));
    expect(r.post).toBe(false);
    expect(r.reason).toContain("is not");
  });

  it("does NOT post for a non-dependabot branch prefix", () => {
    expect(decideBotReviewGate(baseInput({ headBranch: "feature/x" })).post).toBe(false);
  });

  it("does NOT post for a non-pull_request run (e.g. a push build)", () => {
    expect(decideBotReviewGate(baseInput({ event: "push" })).post).toBe(false);
  });

  it("is defensive against a null/empty input", () => {
    expect(decideBotReviewGate(undefined).post).toBe(false);
    expect(decideBotReviewGate({}).post).toBe(false);
  });
});
