import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({ active: false, sleepInhibited: false }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

import { useVigieStore } from "../../store";
import { TitleBar } from "./TitleBar";

describe("TitleBar remote indicator", () => {
  it("shows the active-remote indicator only when remote is active", () => {
    useVigieStore.setState({ remote: { active: false, sleepInhibited: false } });
    const { rerender } = render(<TitleBar />);
    expect(screen.queryByLabelText(/remote active/i)).not.toBeInTheDocument();

    useVigieStore.setState({ remote: { active: true, token: "t", url: "u", sleepInhibited: true } });
    rerender(<TitleBar />);
    expect(screen.getByLabelText(/remote active/i)).toBeInTheDocument();
  });
});
