import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { UsagePanel } from "./UsagePanel";
import * as api from "../../api";
import type { AcpUsageSummary } from "../../api";

vi.mock("../../api", () => ({ getAcpUsageSummary: vi.fn() }));
const mockGet = vi.mocked(api.getAcpUsageSummary);

const SUMMARY: AcpUsageSummary = {
  totalCost: 20.5,
  currency: "USD",
  sessions: 3,
  byModel: [
    {
      provider: "claude-acp",
      model: "opus-4",
      cost: 18.4,
      currency: "USD",
      contextPeak: 250_000,
      contextSize: 1_000_000,
      sessions: 2,
      rateStatus: "rejected",
    },
    {
      provider: "mistral-acp",
      model: null,
      cost: 2.1,
      currency: "USD",
      contextPeak: 40_000,
      contextSize: 128_000,
      sessions: 1,
      rateStatus: null,
    },
  ],
  byTask: [{ taskId: "TASK-92", cost: 20.5, currency: "USD" }],
};

beforeEach(() => {
  mockGet.mockReset();
});

describe("UsagePanel", () => {
  it("fetches the 7-day window on mount and renders per-model spend", async () => {
    mockGet.mockResolvedValue(SUMMARY);
    render(<UsagePanel onClose={vi.fn()} />);

    // "20.50 USD" is both the total and the single task's cost.
    await waitFor(() => expect(screen.getAllByText("20.50 USD").length).toBeGreaterThan(0));
    expect(mockGet).toHaveBeenCalledWith(7);
    expect(screen.getByText("across 3 ACP sessions")).toBeInTheDocument();
    // Model rows + the provider-only (null model) row rendered.
    expect(screen.getByText("opus-4")).toBeInTheDocument();
    expect(screen.getByText("mistral-acp")).toBeInTheDocument();
    // Context peak rendered as a percentage of the window.
    expect(screen.getByText("25% (250,000)")).toBeInTheDocument();
    // A throttled rate status is surfaced.
    expect(screen.getByText("rejected")).toBeInTheDocument();
    // By-task breakdown.
    expect(screen.getByText("TASK-92")).toBeInTheDocument();
  });

  it("re-fetches when the window changes", async () => {
    mockGet.mockResolvedValue(SUMMARY);
    render(<UsagePanel onClose={vi.fn()} />);
    await waitFor(() => expect(mockGet).toHaveBeenCalledWith(7));

    fireEvent.click(screen.getByRole("tab", { name: "30 days" }));
    await waitFor(() => expect(mockGet).toHaveBeenCalledWith(30));
  });

  it("shows an empty state when no ACP usage exists", async () => {
    mockGet.mockResolvedValue({
      totalCost: 0,
      currency: null,
      sessions: 0,
      byModel: [],
      byTask: [],
    });
    render(<UsagePanel onClose={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByText(/No ACP usage recorded in this window/i)).toBeInTheDocument(),
    );
  });

  it("surfaces a load error", async () => {
    mockGet.mockRejectedValue("boom");
    render(<UsagePanel onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/Couldn.t load usage: boom/i)).toBeInTheDocument());
  });
});
