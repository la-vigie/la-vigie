import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DiffPanel } from "./DiffPanel";

// Mock the api module
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

// Simple unified diff text for tests
const SAMPLE_DIFF = `diff --git a/src/foo.ts b/src/foo.ts
index abc..def 100644
--- a/src/foo.ts
+++ b/src/foo.ts
@@ -1,3 +1,4 @@
 const x = 1;
-const y = 2;
+const y = 3;
+const z = 4;
`;

const FILE_CHANGES = [
  { path: "src/foo.ts", change: "modified" },
  { path: "src/bar.ts", change: "added" },
];

// A two-file diff (modified + added) so collapse-all has something to fold.
const TWO_FILE_DIFF = `diff --git a/src/foo.ts b/src/foo.ts
index abc..def 100644
--- a/src/foo.ts
+++ b/src/foo.ts
@@ -1,3 +1,4 @@
 const x = 1;
-const y = 2;
+const y = 3;
+const z = 4;
diff --git a/src/bar.ts b/src/bar.ts
new file mode 100644
index 000..aaa
--- /dev/null
+++ b/src/bar.ts
@@ -0,0 +1,1 @@
+const bar = 99;
`;

// A deleted file: gitdiff-parser leaves newPath as "/dev/null" here.
const DELETED_FILE_DIFF = `diff --git a/src/gone.ts b/src/gone.ts
deleted file mode 100644
index abc..000
--- a/src/gone.ts
+++ /dev/null
@@ -1,2 +0,0 @@
-const a = 1;
-const b = 2;
`;

describe("DiffPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
  });

  it("shows both changed files with their change labels and checkboxes checked by default", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    render(<DiffPanel taskId="task-1" />);

    // Wait for the file list to appear (the file-list span, not the diff header)
    await waitFor(() => {
      const fileList = screen.getByRole("list", { name: /changed files/i });
      expect(fileList).toBeInTheDocument();
      expect(fileList).toHaveTextContent("src/foo.ts");
    });

    const fileList = screen.getByRole("list", { name: /changed files/i });
    expect(fileList).toHaveTextContent("src/bar.ts");
    expect(fileList).toHaveTextContent("modified");
    expect(fileList).toHaveTextContent("added");

    // Checkboxes should be checked by default
    const checkboxes = screen.getAllByRole("checkbox");
    expect(checkboxes).toHaveLength(2);
    for (const cb of checkboxes) {
      expect(cb).toBeChecked();
    }
  });

  it("calls stage_files then commit_task with checked paths and message when Stage & commit clicked", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      if (cmd === "stage_files") return Promise.resolve(undefined);
      if (cmd === "commit_task") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });

    render(<DiffPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("list", { name: /changed files/i })).toHaveTextContent("src/foo.ts");
    });

    const messageInput = screen.getByRole("textbox");
    fireEvent.change(messageInput, { target: { value: "feat: fix things" } });

    const stageCommitBtn = screen.getByRole("button", { name: /^commit$/i });
    fireEvent.click(stageCommitBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("stage_files", {
        taskId: "task-1",
        paths: ["src/foo.ts", "src/bar.ts"],
      });
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("commit_task", {
        taskId: "task-1",
        message: "feat: fix things",
      });
    });
  });

  it("shows an empty-state message when diff is empty and file list is empty", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve("");
      if (cmd === "get_changed_files") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    // Default scope is "uncommitted".
    render(<DiffPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByText("No uncommitted changes")).toBeInTheDocument();
    });
  });

  it('read-only "base" scope shows no checkboxes and no commit box', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    render(<DiffPanel taskId="task-1" scope="base" readOnly />);

    await waitFor(() => {
      expect(screen.getByRole("list", { name: /changed files/i })).toHaveTextContent("src/foo.ts");
    });

    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
    expect(screen.queryByRole("button", { name: /^commit$/i })).not.toBeInTheDocument();
  });

  it("Stage & commit button is disabled when message is empty", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    render(<DiffPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("list", { name: /changed files/i })).toHaveTextContent("src/foo.ts");
    });

    const stageCommitBtn = screen.getByRole("button", { name: /^commit$/i });
    expect(stageCommitBtn).toBeDisabled();
  });

  it("renders diff content via react-diff-view showing added line text and file path header", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    const { container } = render(<DiffPanel taskId="task-1" />);

    // Wait for file list to load (data is ready)
    await waitFor(() => {
      expect(screen.getByRole("list", { name: /changed files/i })).toHaveTextContent("src/foo.ts");
    });

    // File path header rendered by the diff renderer (a collapse toggle button)
    expect(screen.getByRole("button", { name: /src\/foo\.ts/ })).toBeInTheDocument();

    // Added line text: with syntax highlighting active the text is split across token <span>s,
    // so we assert via textContent rather than getByText
    expect(container.textContent).toContain("const y = 3;");

    // Syntax highlighting is active: at least one .token element should be present
    expect(container.querySelector(".token")).not.toBeNull();
  });

  it("does not show commenting affordances when not commentable", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });
    render(<DiffPanel taskId="task-1" />);
    await waitFor(() =>
      expect(screen.getByRole("list", { name: /changed files/i })).toHaveTextContent("src/foo.ts"),
    );
    expect(screen.queryByText(/comments pending/i)).not.toBeInTheDocument();
  });

  it("commentable: clicking a diff line opens a composer; saving shows an inline comment", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });
    const { container } = render(<DiffPanel taskId="task-1" scope="base" readOnly commentable />);
    await waitFor(() =>
      expect(screen.getByRole("list", { name: /changed files/i })).toHaveTextContent("src/foo.ts"),
    );

    // Click the first added code line in the rendered diff.
    const codeCell = container.querySelector(".diff-code-insert") as HTMLElement;
    expect(codeCell).not.toBeNull();
    fireEvent.click(codeCell);

    const box = await screen.findByRole("textbox", { name: /foo\.ts:/i });
    fireEvent.change(box, { target: { value: "rename this" } });
    fireEvent.click(screen.getByRole("button", { name: /add comment/i }));

    expect(await screen.findByText("rename this")).toBeInTheDocument();
    // The count badge is a nested <span> so textContent is split across child elements;
    // use the review-footer__count span directly.
    const countEl = container.querySelector(".review-footer__count");
    expect(countEl?.textContent?.replace(/\s+/g, " ").trim()).toMatch(/1\s*comments pending/i);
  });

  it("Stage & commit button is disabled when no files are checked", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    render(<DiffPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("list", { name: /changed files/i })).toHaveTextContent("src/foo.ts");
    });

    // Uncheck all
    const checkboxes = screen.getAllByRole("checkbox");
    for (const cb of checkboxes) {
      fireEvent.click(cb);
    }

    const messageInput = screen.getByRole("textbox");
    fireEvent.change(messageInput, { target: { value: "some message" } });

    const stageCommitBtn = screen.getByRole("button", { name: /^commit$/i });
    expect(stageCommitBtn).toBeDisabled();
  });

  it("shows the real path (not /dev/null) and a 'deleted' label for a deleted file", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(DELETED_FILE_DIFF);
      if (cmd === "get_changed_files")
        return Promise.resolve([{ path: "src/gone.ts", change: "deleted" }]);
      return Promise.resolve(undefined);
    });

    render(<DiffPanel taskId="task-1" scope="base" readOnly />);

    const header = await screen.findByRole("button", { name: /src\/gone\.ts/ });
    expect(header).toHaveTextContent("src/gone.ts");
    expect(header).toHaveTextContent("deleted");
    expect(header).not.toHaveTextContent("/dev/null");
  });

  it("renders distinct headers (no duplicate React keys) for multiple deleted files", async () => {
    const twoDeleted = `diff --git a/src/one.ts b/src/one.ts
deleted file mode 100644
index abc..000
--- a/src/one.ts
+++ /dev/null
@@ -1,1 +0,0 @@
-const one = 1;
diff --git a/src/two.ts b/src/two.ts
deleted file mode 100644
index def..000
--- a/src/two.ts
+++ /dev/null
@@ -1,1 +0,0 @@
-const two = 2;
`;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(twoDeleted);
      if (cmd === "get_changed_files")
        return Promise.resolve([
          { path: "src/one.ts", change: "deleted" },
          { path: "src/two.ts", change: "deleted" },
        ]);
      return Promise.resolve(undefined);
    });

    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<DiffPanel taskId="task-1" scope="base" readOnly />);

    await screen.findByRole("button", { name: /src\/one\.ts/ });
    expect(screen.getByRole("button", { name: /src\/two\.ts/ })).toBeInTheDocument();

    const dupKeyWarning = errorSpy.mock.calls.some((args) =>
      String(args[0]).includes("same key"),
    );
    expect(dupKeyWarning).toBe(false);
    errorSpy.mockRestore();
  });

  it("collapsing a file hides its hunks but keeps the header visible", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    const { container } = render(<DiffPanel taskId="task-collapse-1" />);

    const header = await screen.findByRole("button", { name: /src\/foo\.ts/ });
    expect(container.textContent).toContain("const y = 3;");
    expect(header).toHaveAttribute("aria-expanded", "true");

    fireEvent.click(header);

    // Header still present, hunks gone.
    expect(screen.getByRole("button", { name: /src\/foo\.ts/ })).toBeInTheDocument();
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(container.textContent).not.toContain("const y = 3;");
  });

  it("collapse-all folds every file; expand-all unfolds them", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(TWO_FILE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    const { container } = render(<DiffPanel taskId="task-collapse-2" />);

    await screen.findByRole("button", { name: /src\/foo\.ts/ });
    expect(container.textContent).toContain("const y = 3;");
    expect(container.textContent).toContain("const bar = 99;");

    fireEvent.click(screen.getByRole("button", { name: /collapse all/i }));
    expect(container.textContent).not.toContain("const y = 3;");
    expect(container.textContent).not.toContain("const bar = 99;");
    // Headers remain.
    expect(screen.getByRole("button", { name: /src\/foo\.ts/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /src\/bar\.ts/ })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /expand all/i }));
    expect(container.textContent).toContain("const y = 3;");
    expect(container.textContent).toContain("const bar = 99;");
  });

  it("persists collapse state across a diff refresh", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_diff") return Promise.resolve(SAMPLE_DIFF);
      if (cmd === "get_changed_files") return Promise.resolve(FILE_CHANGES);
      return Promise.resolve(undefined);
    });

    const { container, rerender } = render(
      <DiffPanel taskId="task-collapse-3" refreshToken={0} />,
    );

    const header = await screen.findByRole("button", { name: /src\/foo\.ts/ });
    fireEvent.click(header);
    expect(container.textContent).not.toContain("const y = 3;");

    // Bump refreshToken to trigger a re-fetch + re-render (simulating a poll).
    rerender(<DiffPanel taskId="task-collapse-3" refreshToken={1} />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /src\/foo\.ts/ })).toHaveAttribute(
        "aria-expanded",
        "false",
      ),
    );
    expect(container.textContent).not.toContain("const y = 3;");
  });
});
