import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { DeleteTaskModal } from "./DeleteTaskModal";
import type { Task } from "../../store";

const task = { id: "t1", title: "Fix login", branch: "task-1-fix-login" } as Task;

describe("DeleteTaskModal", () => {
  it("confirms with deleteBranch=false by default", () => {
    const onConfirm = vi.fn();
    render(<DeleteTaskModal task={task} onCancel={vi.fn()} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(onConfirm).toHaveBeenCalledWith(false);
  });

  it("confirms with deleteBranch=true when the checkbox is checked", () => {
    const onConfirm = vi.fn();
    render(<DeleteTaskModal task={task} onCancel={vi.fn()} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(onConfirm).toHaveBeenCalledWith(true);
  });

  it("Cancel calls onCancel and not onConfirm", () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(<DeleteTaskModal task={task} onCancel={onCancel} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("Escape calls onCancel", () => {
    const onCancel = vi.fn();
    render(<DeleteTaskModal task={task} onCancel={onCancel} onConfirm={vi.fn()} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("Delete button is re-enabled after onConfirm resolves (busy reset on success)", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(<DeleteTaskModal task={task} onCancel={vi.fn()} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Delete" })).not.toBeDisabled()
    );
  });
});
