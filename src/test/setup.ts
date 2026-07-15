import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";

// TASK-189: The default `waitFor`/`findBy*` timeout is 1000ms. Heavy component flows
// (e.g. PrPanel's create round-trip, TaskDetail's finish flow) legitimately complete just
// under that bound on an unloaded machine, but occasionally exceed it under CI resource
// contention / unlucky file ordering — surfacing as a rare, re-run-green flake where a
// correct assertion (`toHaveBeenCalledWith("create_pr", …)`) simply hasn't been reached in
// time. Widen the async-utility timeout so transient slowness can't fail a correct test.
// Genuine failures still fail (just a few seconds slower); this only adds headroom.
configure({ asyncUtilTimeout: 3000 });

// On newer Node versions an experimental global `localStorage` exists but is
// `undefined` (it needs `--localstorage-file`), and it shadows jsdom's
// `window.localStorage`. Modules that read localStorage at import time (the
// Zustand store) then throw. Install a minimal in-memory implementation so the
// suite runs regardless of the Node version.
if (!window.localStorage || typeof window.localStorage.getItem !== "function") {
  const store: Record<string, string> = {};
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, value: string) => {
        store[key] = String(value);
      },
      removeItem: (key: string) => {
        delete store[key];
      },
      clear: () => {
        for (const key of Object.keys(store)) delete store[key];
      },
      key: (index: number) => Object.keys(store)[index] ?? null,
      get length() {
        return Object.keys(store).length;
      },
    },
  });
}

afterEach(() => {
  cleanup();
});
