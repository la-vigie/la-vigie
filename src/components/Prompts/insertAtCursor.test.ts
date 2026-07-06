import { describe, it, expect } from "vitest";
import { insertAtCursor } from "./insertAtCursor";

describe("insertAtCursor", () => {
  it("inserts at a collapsed cursor", () => {
    const r = insertAtCursor("Use the API. ", 13, 13, "Go now");
    expect(r.value).toBe("Use the API. Go now");
    expect(r.cursor).toBe(19);
  });

  it("replaces a selection", () => {
    const r = insertAtCursor("keep XXX tail", 5, 8, "YES");
    expect(r.value).toBe("keep YES tail");
    expect(r.cursor).toBe(8);
  });

  it("appends when cursor is at end of empty string", () => {
    const r = insertAtCursor("", 0, 0, "hello");
    expect(r.value).toBe("hello");
    expect(r.cursor).toBe(5);
  });
});
