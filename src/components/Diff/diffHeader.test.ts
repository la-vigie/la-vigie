import { describe, expect, it } from "vitest";
import { displayPath, fileChangeLabel } from "./diffHeader";

describe("displayPath", () => {
  it("uses newPath for a modified file", () => {
    expect(displayPath({ type: "modify", oldPath: "src/foo.ts", newPath: "src/foo.ts" })).toBe(
      "src/foo.ts",
    );
  });

  it("uses newPath for an added file (oldPath is /dev/null)", () => {
    expect(displayPath({ type: "add", oldPath: "/dev/null", newPath: "src/bar.ts" })).toBe(
      "src/bar.ts",
    );
  });

  it("uses oldPath for a deleted file (newPath is /dev/null)", () => {
    expect(displayPath({ type: "delete", oldPath: "src/gone.ts", newPath: "/dev/null" })).toBe(
      "src/gone.ts",
    );
  });

  it("uses newPath for a renamed file", () => {
    expect(displayPath({ type: "rename", oldPath: "src/old.ts", newPath: "src/new.ts" })).toBe(
      "src/new.ts",
    );
  });

  it("uses newPath for a copied file", () => {
    expect(displayPath({ type: "copy", oldPath: "src/src.ts", newPath: "src/dest.ts" })).toBe(
      "src/dest.ts",
    );
  });

  it("falls back to 'unknown' when both sides are missing", () => {
    expect(displayPath({ type: "modify" })).toBe("unknown");
  });
});

describe("fileChangeLabel", () => {
  it("labels added/deleted/renamed/copied", () => {
    expect(fileChangeLabel("add")).toBe("added");
    expect(fileChangeLabel("delete")).toBe("deleted");
    expect(fileChangeLabel("rename")).toBe("renamed");
    expect(fileChangeLabel("copy")).toBe("copied");
  });

  it("returns null for unknown/undefined types", () => {
    expect(fileChangeLabel(undefined)).toBeNull();
    expect(fileChangeLabel("type_changed")).toBeNull();
  });

  it("returns null for a plain modification (no badge)", () => {
    expect(fileChangeLabel("modify")).toBeNull();
  });
});
