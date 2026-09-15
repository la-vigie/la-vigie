import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { exposeWdioOriginalCore, installWdioCoreHook } from "./exposeWdioCore";

describe("exposeWdioOriginalCore", () => {
  it("aliases __TAURI__.core onto __wdio_original_core__ when the Tauri global is present", () => {
    const core = { invoke: () => {} };
    const win: Record<string, unknown> = { __TAURI__: { core } };

    expect(exposeWdioOriginalCore(win)).toBe(true);
    // The exact object the service's guest script reads for `.invoke`.
    expect(win.__wdio_original_core__).toBe(core);
  });

  it("is a no-op when the Tauri global is absent (production)", () => {
    const win: Record<string, unknown> = {};

    expect(exposeWdioOriginalCore(win)).toBe(false);
    expect(win.__wdio_original_core__).toBeUndefined();
  });

  it("is a no-op when __TAURI__.core lacks a callable invoke", () => {
    const win: Record<string, unknown> = { __TAURI__: { core: { invoke: 42 } } };

    expect(exposeWdioOriginalCore(win)).toBe(false);
    expect(win.__wdio_original_core__).toBeUndefined();
  });

  it("is idempotent — a pre-existing hook is reported ready and left untouched", () => {
    const existing = { invoke: () => {} };
    const core = { invoke: () => {} };
    const win: Record<string, unknown> = {
      __wdio_original_core__: existing,
      __TAURI__: { core },
    };

    expect(exposeWdioOriginalCore(win)).toBe(true);
    expect(win.__wdio_original_core__).toBe(existing);
  });
});

describe("installWdioCoreHook", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("installs synchronously when the global is already present (no timer)", () => {
    const core = { invoke: () => {} };
    const win: Record<string, unknown> = { __TAURI__: { core } };
    const setInterval = vi.spyOn(globalThis, "setInterval");

    installWdioCoreHook(win);

    expect(win.__wdio_original_core__).toBe(core);
    expect(setInterval).not.toHaveBeenCalled();
  });

  it("retries until the asynchronously-injected global appears, then stops", () => {
    const win: Record<string, unknown> = {};
    const clearInterval = vi.spyOn(globalThis, "clearInterval");

    installWdioCoreHook(win, { intervalMs: 50, maxWaitMs: 10000 });
    expect(win.__wdio_original_core__).toBeUndefined();

    // Tauri injects the global after page load.
    const core = { invoke: () => {} };
    win.__TAURI__ = { core };
    vi.advanceTimersByTime(50);

    expect(win.__wdio_original_core__).toBe(core);
    expect(clearInterval).toHaveBeenCalled();
  });

  it("gives up after the budget so it can never spin forever (production)", () => {
    const win: Record<string, unknown> = {};
    const clearInterval = vi.spyOn(globalThis, "clearInterval");

    installWdioCoreHook(win, { intervalMs: 50, maxWaitMs: 200 });
    // 200 / 50 = 4 attempts; advance past the budget.
    vi.advanceTimersByTime(1000);

    expect(win.__wdio_original_core__).toBeUndefined();
    expect(clearInterval).toHaveBeenCalled();
  });
});
