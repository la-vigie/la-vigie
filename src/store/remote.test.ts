import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useVigieStore } from "./index";

describe("remote control store slice", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    useVigieStore.setState({ remote: { active: false, sleepInhibited: false } });
  });

  it("enableRemoteControl stores the returned active status + token", async () => {
    invokeMock.mockResolvedValueOnce({ active: true, token: "abc", url: "https://mac.ts.net/", sleepInhibited: true });
    await useVigieStore.getState().enableRemoteControl();
    expect(invokeMock).toHaveBeenCalledWith("enable_remote");
    expect(useVigieStore.getState().remote).toEqual({ active: true, token: "abc", url: "https://mac.ts.net/", sleepInhibited: true });
  });

  it("disableRemoteControl resets to inactive", async () => {
    useVigieStore.setState({ remote: { active: true, token: "abc", url: "u", sleepInhibited: true } });
    invokeMock.mockResolvedValueOnce({ active: false, token: null, url: null, sleepInhibited: false });
    await useVigieStore.getState().disableRemoteControl();
    expect(invokeMock).toHaveBeenCalledWith("disable_remote");
    expect(useVigieStore.getState().remote.active).toBe(false);
  });
});
