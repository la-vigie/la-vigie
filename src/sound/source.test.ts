import { describe, it, expect, vi, beforeEach } from "vitest";

const readSoundBytes = vi.fn();
vi.mock("../api", () => ({ readSoundBytes: (id: string) => readSoundBytes(id) }));

import { getSoundUrl, resetSoundUrlCache } from "./source";
import type { CustomSound } from "./types";

const custom: CustomSound[] = [{ id: "custom:abc", label: "Ding", ext: "mp3" }];

beforeEach(() => {
  readSoundBytes.mockReset();
  resetSoundUrlCache();
  // jsdom lacks createObjectURL.
  (globalThis.URL as unknown as { createObjectURL: () => string }).createObjectURL = vi
    .fn()
    .mockReturnValue("blob:fake");
});

describe("getSoundUrl", () => {
  it("returns the static path for a bundled id without touching IPC", async () => {
    const url = await getSoundUrl("jobs-done", custom);
    expect(url).toBe("/sounds/jobs-done.mp3");
    expect(readSoundBytes).not.toHaveBeenCalled();
  });

  it("builds and caches a blob URL for a custom id", async () => {
    readSoundBytes.mockResolvedValue([1, 2, 3]);
    const first = await getSoundUrl("custom:abc", custom);
    const second = await getSoundUrl("custom:abc", custom);
    expect(first).toBe("blob:fake");
    expect(second).toBe("blob:fake");
    expect(readSoundBytes).toHaveBeenCalledTimes(1); // cached
  });

  it("returns null for an unknown id", async () => {
    const url = await getSoundUrl("custom:gone", custom);
    expect(url).toBeNull();
    expect(readSoundBytes).not.toHaveBeenCalled();
  });

  it("returns null when readSoundBytes rejects, and does not cache the failure", async () => {
    readSoundBytes.mockRejectedValueOnce(new Error("missing file"));
    expect(await getSoundUrl("custom:abc", custom)).toBeNull();
    // not cached: a subsequent successful read still resolves
    readSoundBytes.mockResolvedValueOnce([1, 2, 3]);
    expect(await getSoundUrl("custom:abc", custom)).toBe("blob:fake");
    expect(readSoundBytes).toHaveBeenCalledTimes(2);
  });
});
