import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useCollapsedFiles } from "./useCollapsedFiles";

describe("useCollapsedFiles", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("starts with nothing collapsed", () => {
    const { result } = renderHook(() => useCollapsedFiles("task-1"));
    expect(result.current.isCollapsed("a.ts")).toBe(false);
  });

  it("toggles a single file", () => {
    const { result } = renderHook(() => useCollapsedFiles("task-1"));
    act(() => result.current.toggle("a.ts"));
    expect(result.current.isCollapsed("a.ts")).toBe(true);
    act(() => result.current.toggle("a.ts"));
    expect(result.current.isCollapsed("a.ts")).toBe(false);
  });

  it("collapseAll collapses the given paths; expandAll clears them", () => {
    const { result } = renderHook(() => useCollapsedFiles("task-1"));
    act(() => result.current.collapseAll(["a.ts", "b.ts"]));
    expect(result.current.isCollapsed("a.ts")).toBe(true);
    expect(result.current.isCollapsed("b.ts")).toBe(true);
    act(() => result.current.expandAll());
    expect(result.current.isCollapsed("a.ts")).toBe(false);
    expect(result.current.isCollapsed("b.ts")).toBe(false);
  });

  it("persists collapse state across remounts (survives diff refresh / re-render)", () => {
    const first = renderHook(() => useCollapsedFiles("task-1"));
    act(() => first.result.current.toggle("a.ts"));
    first.unmount();

    const second = renderHook(() => useCollapsedFiles("task-1"));
    expect(second.result.current.isCollapsed("a.ts")).toBe(true);
  });

  it("scopes collapse state per task id", () => {
    const t1 = renderHook(() => useCollapsedFiles("task-1"));
    act(() => t1.result.current.toggle("a.ts"));
    const t2 = renderHook(() => useCollapsedFiles("task-2"));
    expect(t2.result.current.isCollapsed("a.ts")).toBe(false);
  });
});
