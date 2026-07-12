import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useVigieStore } from "./index";

describe("inject-lavigie-skills store slice", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    useVigieStore.setState({ injectLavigieSkills: false });
  });

  it("optimistically sets state and calls the api", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await useVigieStore.getState().setInjectLavigieSkills(true);

    expect(useVigieStore.getState().injectLavigieSkills).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("set_inject_lavigie_skills", { enabled: true });
  });
});
