import { describe, it, expect } from "vitest";
import { finishModalView } from "./finishModalView";
import type { PrStatus } from "../../api";
import type { Task } from "../../store";

const baseTask = { branch: "task-39-finish", baseBranch: "main", inPlace: false } as Pick<
  Task,
  "branch" | "baseBranch" | "inPlace"
>;

const pr = (state: string, number = 7): PrStatus => ({
  number,
  url: `https://github.com/foo/bar/pull/${number}`,
  title: "T",
  state,
  isDraft: false,
  mergeable: "MERGEABLE",
  reviewDecision: null,
  checks: [],
});

describe("finishModalView", () => {
  it("carries branch + base through", () => {
    const v = finishModalView(baseTask, null, 0);
    expect(v.branch).toBe("task-39-finish");
    expect(v.baseBranch).toBe("main");
  });

  describe("PR state normalization + primary selection", () => {
    it("OPEN → merge is offered and primary", () => {
      const v = finishModalView(baseTask, pr("OPEN"), 0);
      expect(v.prState).toBe("open");
      expect(v.prText).toBe("#7 open");
      expect(v.showMerge).toBe(true);
      expect(v.primaryMode).toBe("merge");
    });

    it("MERGED → no merge, keep is primary", () => {
      const v = finishModalView(baseTask, pr("MERGED"), 0);
      expect(v.prState).toBe("merged");
      expect(v.prText).toBe("#7 merged");
      expect(v.showMerge).toBe(false);
      expect(v.primaryMode).toBe("keep");
    });

    it("CLOSED → other, no merge, keep primary", () => {
      const v = finishModalView(baseTask, pr("CLOSED"), 0);
      expect(v.prState).toBe("other");
      expect(v.prText).toBe("#7 closed");
      expect(v.showMerge).toBe(false);
    });

    it("no PR → none, no merge, keep primary", () => {
      const v = finishModalView(baseTask, null, 0);
      expect(v.prState).toBe("none");
      expect(v.prText).toBe("none");
      expect(v.showMerge).toBe(false);
      expect(v.primaryMode).toBe("keep");
    });
  });

  describe("uncommitted-changes text", () => {
    it("loading (null count) → checking…", () => {
      const v = finishModalView(baseTask, null, null);
      expect(v.dirtyLoading).toBe(true);
      expect(v.dirtyText).toBe("checking…");
    });

    it("0 → clean", () => {
      expect(finishModalView(baseTask, null, 0).dirtyText).toBe("working tree clean");
    });

    it("1 → singular", () => {
      expect(finishModalView(baseTask, null, 1).dirtyText).toBe(
        "1 uncommitted change — kept in the branch",
      );
    });

    it("N → plural", () => {
      expect(finishModalView(baseTask, null, 3).dirtyText).toBe(
        "3 uncommitted changes — kept in the branch",
      );
    });
  });

  describe("discard visibility (TASK-163)", () => {
    it("worktree task → discard shown", () => {
      expect(finishModalView(baseTask, null, 0).showDiscard).toBe(true);
    });

    it("in-place task → discard hidden", () => {
      const v = finishModalView({ ...baseTask, inPlace: true }, null, 0);
      expect(v.showDiscard).toBe(false);
    });
  });
});
