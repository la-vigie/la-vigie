import { beforeEach, describe, expect, it } from "vitest";
import { useVigieStore } from "./index";

describe("setup state", () => {
  beforeEach(() => {
    useVigieStore.setState({ setupByTask: {} });
  });

  it("appends output, creating an entry if needed", () => {
    useVigieStore.getState().appendSetupOutput("t1", "line 1\n");
    useVigieStore.getState().appendSetupOutput("t1", "line 2\n");
    expect(useVigieStore.getState().setupByTask["t1"].log).toBe("line 1\nline 2\n");
    expect(useVigieStore.getState().setupByTask["t1"].status).toBe("running");
  });

  it("sets status without dropping the log", () => {
    useVigieStore.getState().appendSetupOutput("t1", "out\n");
    useVigieStore.getState().setSetupStatus("t1", "failed");
    expect(useVigieStore.getState().setupByTask["t1"]).toEqual({ status: "failed", log: "out\n" });
  });

  it("hydrate replaces status + log", () => {
    useVigieStore.getState().hydrateSetup("t1", "succeeded", "full log");
    expect(useVigieStore.getState().setupByTask["t1"]).toEqual({ status: "succeeded", log: "full log" });
  });
});
