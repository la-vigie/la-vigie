import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RunStatePill } from "./RunStatePill";

describe("RunStatePill", () => {
  it("renders nothing when the agent has exited", () => {
    const { container } = render(<RunStatePill status="exited" onStop={() => {}} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders a Working… status with a Stop-labelled button while working", () => {
    render(<RunStatePill status="running" activity="working" onStop={() => {}} />);
    const btn = screen.getByRole("button", { name: /stop agent/i });
    expect(btn).toHaveTextContent("Working…");
    expect(btn).toHaveTextContent("Stop");
    expect(btn.className).toContain("run-pill--working");
  });

  it("renders Ready, Needs input, and Starting… for their states", () => {
    const { rerender } = render(<RunStatePill status="running" activity="idle" onStop={() => {}} />);
    expect(screen.getByRole("button")).toHaveTextContent("Ready");

    rerender(<RunStatePill status="running" activity="needs_attention" onStop={() => {}} />);
    expect(screen.getByRole("button")).toHaveTextContent("Needs input");

    rerender(<RunStatePill status="starting" onStop={() => {}} />);
    expect(screen.getByRole("button")).toHaveTextContent("Starting…");
  });

  it("calls onStop when the pill is clicked", () => {
    const onStop = vi.fn();
    render(<RunStatePill status="running" activity="working" onStop={onStop} />);
    fireEvent.click(screen.getByRole("button", { name: /stop agent/i }));
    expect(onStop).toHaveBeenCalledTimes(1);
  });
});
