import { describe, expect, it } from "vitest";
import { createNotificationRegistry } from "./registry";

describe("createNotificationRegistry", () => {
  it("assigns distinct ids and resolves them back to task ids", () => {
    const reg = createNotificationRegistry();
    const a = reg.register("task-a");
    const b = reg.register("task-b");
    expect(a).not.toBe(b);
    expect(reg.resolve(a)).toBe("task-a");
    expect(reg.resolve(b)).toBe("task-b");
  });

  it("returns undefined for an unknown id", () => {
    const reg = createNotificationRegistry();
    expect(reg.resolve(999)).toBeUndefined();
  });

  it("keeps ids within the 32-bit positive range", () => {
    const reg = createNotificationRegistry();
    const id = reg.register("task-a");
    expect(id).toBeGreaterThan(0);
    expect(id).toBeLessThanOrEqual(0x7fffffff);
  });
});
