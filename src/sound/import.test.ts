import { describe, it, expect, vi, beforeEach } from "vitest";

const open = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => open(...a) }));

const importCustomSound = vi.fn();
const deleteCustomSound = vi.fn();
vi.mock("../api", () => ({
  importCustomSound: (...a: unknown[]) => importCustomSound(...a),
  deleteCustomSound: (...a: unknown[]) => deleteCustomSound(...a),
}));

const getSoundUrl = vi.fn();
const decodeTest = vi.fn();
vi.mock("./source", () => ({
  getSoundUrl: (...a: unknown[]) => getSoundUrl(...a),
  decodeTest: (...a: unknown[]) => decodeTest(...a),
}));

import { pickAndImportSound } from "./import";

beforeEach(() => {
  open.mockReset();
  importCustomSound.mockReset();
  deleteCustomSound.mockReset();
  getSoundUrl.mockReset();
  decodeTest.mockReset();
});

describe("pickAndImportSound", () => {
  it("returns null when the picker is cancelled", async () => {
    open.mockResolvedValue(null);
    expect(await pickAndImportSound()).toBeNull();
    expect(importCustomSound).not.toHaveBeenCalled();
  });

  it("imports and returns the entry when decode succeeds", async () => {
    open.mockResolvedValue("/music/ding.mp3");
    const entry = { id: "custom:abc", label: "ding", ext: "mp3" };
    importCustomSound.mockResolvedValue(entry);
    getSoundUrl.mockResolvedValue("blob:fake");
    decodeTest.mockResolvedValue(true);

    expect(await pickAndImportSound()).toEqual(entry);
    expect(importCustomSound).toHaveBeenCalledWith("/music/ding.mp3", "ding");
    expect(deleteCustomSound).not.toHaveBeenCalled();
  });

  it("rolls back and throws when decode fails", async () => {
    open.mockResolvedValue("/music/broken.mp3");
    const entry = { id: "custom:bad", label: "broken", ext: "mp3" };
    importCustomSound.mockResolvedValue(entry);
    getSoundUrl.mockResolvedValue("blob:fake");
    decodeTest.mockResolvedValue(false);

    await expect(pickAndImportSound()).rejects.toThrow(/could not be played/i);
    expect(deleteCustomSound).toHaveBeenCalledWith("custom:bad");
  });
});
