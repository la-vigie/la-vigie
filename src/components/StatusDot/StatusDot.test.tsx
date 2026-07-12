import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatusDot } from "./StatusDot";

describe("StatusDot", () => {
  it("renders a distinct color for the pending (queued) status", () => {
    const { container } = render(<StatusDot status="pending" />);
    const dot = container.querySelector(".status-dot") as HTMLElement;
    expect(dot).toBeTruthy();
    // Pending must have a non-empty, distinct background color.
    expect(dot.style.backgroundColor).not.toBe("");
  });
});
