import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SchedulesTab } from "./SchedulesTab";
import * as api from "../../api";

vi.mock("../../api");

const sample: api.Schedule = {
  id: "s1", repoId: "r1", name: "Weekly scan", prompt: "/security-scan",
  cron: "0 7 * * 1", agent: null, model: null, baseBranch: null,
  enabled: true, nextRunAt: 1_800_000_000, lastRunAt: null,
  createdAt: 1, updatedAt: 1,
  oneShot: false,
  skipRepoPrompt: false,
};

beforeEach(() => {
  vi.mocked(api.listSchedules).mockResolvedValue([sample]);
  vi.mocked(api.previewNextRun).mockResolvedValue(1_800_000_000);
  vi.mocked(api.createSchedule).mockResolvedValue({ ...sample, id: "s2", name: "Nightly" });
  vi.mocked(api.deleteSchedule).mockResolvedValue(undefined);
  vi.mocked(api.updateSchedule).mockResolvedValue({ ...sample, name: "Renamed" });
  vi.mocked(api.createOneShotSchedule).mockResolvedValue({
    ...sample, id: "s3", name: "Quota resume", oneShot: true, cron: "",
  });
});

describe("SchedulesTab", () => {
  it("lists existing schedules for the repo", async () => {
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    expect(await screen.findByText("Weekly scan")).toBeInTheDocument();
    expect(api.listSchedules).toHaveBeenCalledWith("r1");
  });

  it("creates a schedule from the form", async () => {
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    await screen.findByText("Weekly scan");

    await userEvent.type(screen.getByLabelText("Schedule name"), "Nightly");
    await userEvent.type(screen.getByLabelText("Prompt"), "/nightly");
    await userEvent.clear(screen.getByLabelText("Cron"));
    await userEvent.type(screen.getByLabelText("Cron"), "0 2 * * *");
    await userEvent.click(screen.getByRole("button", { name: "Add schedule" }));

    await waitFor(() =>
      expect(api.createSchedule).toHaveBeenCalledWith(
        expect.objectContaining({
          repoId: "r1", name: "Nightly", prompt: "/nightly", cron: "0 2 * * *",
          // TASK-181: the form defaults to skip = true.
          skipRepoPrompt: true,
        }),
      ),
    );
  });

  it("passes the skip-repo-prompt choice when unchecked (TASK-181)", async () => {
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    await screen.findByText("Weekly scan");

    await userEvent.type(screen.getByLabelText("Schedule name"), "Nightly");
    await userEvent.type(screen.getByLabelText("Prompt"), "/nightly");
    await userEvent.click(screen.getByLabelText("Skip repository prompt")); // uncheck
    await userEvent.click(screen.getByRole("button", { name: "Add schedule" }));

    await waitFor(() =>
      expect(api.createSchedule).toHaveBeenCalledWith(
        expect.objectContaining({ name: "Nightly", skipRepoPrompt: false }),
      ),
    );
  });

  it("deletes a schedule", async () => {
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    await screen.findByText("Weekly scan");
    await userEvent.click(screen.getByRole("button", { name: "Delete Weekly scan" }));
    await waitFor(() => expect(api.deleteSchedule).toHaveBeenCalledWith("s1"));
  });

  it("edits an existing schedule", async () => {
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    await screen.findByText("Weekly scan");

    await userEvent.click(screen.getByRole("button", { name: "Edit Weekly scan" }));

    const nameInput = screen.getByLabelText("Schedule name") as HTMLInputElement;
    expect(nameInput.value).toBe("Weekly scan");
    // TASK-181: the checkbox reflects the stored value (sample has skip = false).
    const skipBox = screen.getByLabelText("Skip repository prompt") as HTMLInputElement;
    expect(skipBox.checked).toBe(false);

    await userEvent.clear(nameInput);
    await userEvent.type(nameInput, "Renamed");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(api.updateSchedule).toHaveBeenCalledWith(
        expect.objectContaining({ id: "s1", name: "Renamed", enabled: true, skipRepoPrompt: false }),
      ),
    );
  });

  it("creates a one-time schedule from a relative delay", async () => {
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    await screen.findByText("Weekly scan");

    await userEvent.click(screen.getByRole("radio", { name: "One-time" }));
    await userEvent.type(screen.getByLabelText("Schedule name"), "Quota resume");
    await userEvent.type(screen.getByLabelText("Prompt"), "/resume");
    await userEvent.clear(screen.getByLabelText("In hours"));
    await userEvent.type(screen.getByLabelText("In hours"), "3");
    await userEvent.click(screen.getByRole("button", { name: "Add schedule" }));

    await waitFor(() =>
      expect(api.createOneShotSchedule).toHaveBeenCalledWith(
        expect.objectContaining({ repoId: "r1", name: "Quota resume", prompt: "/resume", inSeconds: 10800 }),
      ),
    );
  });

  it("shows one-time rows with a fire time instead of a cron", async () => {
    vi.mocked(api.listSchedules).mockResolvedValue([
      { ...sample, id: "os1", name: "One shot", oneShot: true, cron: "", nextRunAt: 1_800_000_000 },
    ]);
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    expect(await screen.findByText("One shot")).toBeInTheDocument();
    // The cron text "0 7 * * 1" must NOT render for a one-shot row.
    expect(screen.queryByText("0 7 * * 1")).not.toBeInTheDocument();
  });

  it("hides the Edit button on a one-shot row but keeps Delete", async () => {
    vi.mocked(api.listSchedules).mockResolvedValue([
      { ...sample, id: "os1", name: "One shot", oneShot: true, cron: "", nextRunAt: 1_800_000_000 },
    ]);
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    await screen.findByText("One shot");

    expect(screen.queryByRole("button", { name: /Edit/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete One shot" })).toBeInTheDocument();
  });

  it("rejects an empty 'In hours' value and does not create a one-shot schedule", async () => {
    // Mock call counts aren't reset between tests in this file (no
    // clearAllMocks); clear explicitly so an earlier test's call doesn't
    // shadow the "not called" assertion below.
    vi.mocked(api.createOneShotSchedule).mockClear();
    render(<SchedulesTab repoId="r1" defaultBranch="main" />);
    await screen.findByText("Weekly scan");

    await userEvent.click(screen.getByRole("radio", { name: "One-time" }));
    await userEvent.type(screen.getByLabelText("Schedule name"), "Quota resume");
    await userEvent.type(screen.getByLabelText("Prompt"), "/resume");
    await userEvent.clear(screen.getByLabelText("In hours"));
    await userEvent.click(screen.getByRole("button", { name: "Add schedule" }));

    // Scope by the error paragraph's class — the "In hours" hint shows the
    // same copy while the value is invalid, so a plain text match is ambiguous.
    await waitFor(() =>
      expect(
        screen.getByText("Enter a positive number of hours.", { selector: ".repo-settings__error" }),
      ).toBeInTheDocument(),
    );
    expect(api.createOneShotSchedule).not.toHaveBeenCalled();
  });
});
