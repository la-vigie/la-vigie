import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

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
