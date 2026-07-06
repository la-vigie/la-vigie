import { renderHook, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useAgents, useAgentModels } from "./useAgents";
import * as api from "../api";

vi.mock("../api");

describe("useAgents", () => {
  beforeEach(() => vi.clearAllMocks());
  it("loads agents once", async () => {
    (api.listAgents as any).mockResolvedValue([{ name: "opencode", displayName: "OpenCode" }]);
    const { result } = renderHook(() => useAgents());
    await waitFor(() => expect(result.current.agents).toHaveLength(1));
    expect(result.current.error).toBeNull();
  });
});

describe("useAgentModels", () => {
  beforeEach(() => vi.clearAllMocks());
  it("loads models for the given agent", async () => {
    (api.listAgentModels as any).mockResolvedValue(["zhipuai-coding-plan/glm-5.2"]);
    const { result } = renderHook(() => useAgentModels("opencode"));
    await waitFor(() => expect(result.current.models).toHaveLength(1));
  });
  it("returns no models for undefined agent", async () => {
    const { result } = renderHook(() => useAgentModels(undefined));
    expect(result.current.models).toEqual([]);
    expect(api.listAgentModels).not.toHaveBeenCalled();
  });
});
