import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { debounce, keyedDebounce, throttleLeading } from "./debounce";

describe("debounce", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("fires once after the window, coalescing rapid calls", () => {
    const fn = vi.fn();
    const d = debounce(fn, 1000);
    d(); d(); d();
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledOnce();
  });

  it("cancel() prevents a pending call", () => {
    const fn = vi.fn();
    const d = debounce(fn, 1000);
    d();
    d.cancel();
    vi.advanceTimersByTime(1000);
    expect(fn).not.toHaveBeenCalled();
  });
});

describe("keyedDebounce", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("keeps a separate timer per key", () => {
    const fn = vi.fn();
    const d = keyedDebounce(fn, 1000);
    d("a"); d("a"); d("b");
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledTimes(2);
    expect(fn).toHaveBeenCalledWith("a");
    expect(fn).toHaveBeenCalledWith("b");
  });
});

describe("throttleLeading", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("fires immediately then ignores calls within the window", () => {
    const fn = vi.fn();
    const t = throttleLeading(fn, 5000);
    t(); t(); t();
    expect(fn).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(5000);
    t();
    expect(fn).toHaveBeenCalledTimes(2);
  });
});
