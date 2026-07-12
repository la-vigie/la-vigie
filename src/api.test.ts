import { beforeEach, describe, expect, it, vi } from "vitest";
import { createTask, checkWorktreePath, removeRepo, startAgent, writeSession, resizeSession, stopSession, onAgentStatus, getDiff, getChangedFiles, stageFiles, commitTask, finishTask, ghStatus, createPr, getPrStatus, getPrComments, openUrl, listAgents, setTaskAgent, upsertCustomAgent, deleteCustomAgent, listTaskDocs, readTaskDoc, setSoundSettings, notifyAgentEvent, listAgentModels, setTaskModel, createPrompt, reorderPrompts } from "./api";

const { invokeMock, ChannelMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  ChannelMock: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: ChannelMock,
}));

// Mock for @tauri-apps/api/event — expose a way to push events
const { listenMock, pushEvent, clearHandlers } = vi.hoisted(() => {
  const handlers: Array<(payload: unknown) => void> = [];
  return {
    listenMock: vi.fn((_event: string, handler: (e: { payload: unknown }) => void) => {
      handlers.push((payload) => handler({ payload }));
      return Promise.resolve(() => {});
    }),
    pushEvent: (payload: unknown) => {
      for (const h of handlers) h(payload);
    },
    clearHandlers: () => {
      handlers.length = 0;
    },
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

// Mock for @tauri-apps/plugin-notification
const { isPermissionGrantedMock, sendNotificationMock, requestPermissionMock, onActionMock } = vi.hoisted(() => ({
  isPermissionGrantedMock: vi.fn(),
  sendNotificationMock: vi.fn(),
  requestPermissionMock: vi.fn(),
  onActionMock: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: isPermissionGrantedMock,
  requestPermission: requestPermissionMock,
  sendNotification: sendNotificationMock,
  onAction: onActionMock,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    setFocus: vi.fn().mockResolvedValue(undefined),
    unminimize: vi.fn().mockResolvedValue(undefined),
    onFocusChanged: vi.fn().mockResolvedValue(() => {}),
    isFocused: vi.fn().mockResolvedValue(false),
  }),
}));

// Mock for @tauri-apps/plugin-opener
const { openUrlMock } = vi.hoisted(() => ({
  openUrlMock: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: openUrlMock,
}));

describe("createTask", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("createTask invokes create_task with repoId, title, baseBranch", async () => {
    invokeMock.mockResolvedValueOnce({ id: "t1" });
    const result = await createTask("repo-1", "My task", "main");

    expect(invokeMock).toHaveBeenCalledWith("create_task", {
      repoId: "repo-1",
      title: "My task",
      baseBranch: "main",
      ticketKey: null,
      agent: null,
      model: null,
      autoApprove: null,
    });
    expect(result).toEqual({ id: "t1" });
  });
});

describe("checkWorktreePath", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("invokes check_worktree_path with camelCase args (nulls for omitted) and returns the preview", async () => {
    invokeMock.mockResolvedValueOnce({ state: "adopt", path: "/wt/r/x", message: "reused" });
    const result = await checkWorktreePath("repo-1", "My task");

    expect(invokeMock).toHaveBeenCalledWith("check_worktree_path", {
      repoId: "repo-1",
      title: "My task",
      baseBranch: null,
      ticketKey: null,
    });
    expect(result).toEqual({ state: "adopt", path: "/wt/r/x", message: "reused" });
  });

  it("forwards baseBranch and ticketKey when provided", async () => {
    invokeMock.mockResolvedValueOnce({ state: "vacant", path: "", message: null });
    await checkWorktreePath("repo-1", "My task", "develop", "TASK-125");

    expect(invokeMock).toHaveBeenCalledWith("check_worktree_path", {
      repoId: "repo-1",
      title: "My task",
      baseBranch: "develop",
      ticketKey: "TASK-125",
    });
  });
});

describe("removeRepo", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("removeRepo invokes remove_repo with repoId", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await removeRepo("repo-1");

    expect(invokeMock).toHaveBeenCalledWith("remove_repo", { repoId: "repo-1" });
  });
});

describe("agent api wrappers", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("startAgent invokes start_agent with taskId, resume, and onEvent channel", async () => {
    invokeMock.mockResolvedValueOnce("agent-1");
    const channel = new ChannelMock();

    const agentId = await startAgent("task-1", false, channel as never);

    expect(invokeMock).toHaveBeenCalledWith("start_agent", {
      taskId: "task-1",
      resume: false,
      initialPrompt: null,
      onEvent: channel,
    });
    expect(agentId).toBe("agent-1");
  });

  it("writeSession invokes write_session with sessionId and data", async () => {
    await writeSession("agent-1", "hello");

    expect(invokeMock).toHaveBeenCalledWith("write_session", {
      sessionId: "agent-1",
      data: "hello",
    });
  });

  it("resizeSession invokes resize_session with sessionId, cols, rows", async () => {
    await resizeSession("agent-1", 80, 24);

    expect(invokeMock).toHaveBeenCalledWith("resize_session", {
      sessionId: "agent-1",
      cols: 80,
      rows: 24,
    });
  });

  it("stopSession invokes stop_session with sessionId", async () => {
    await stopSession("agent-1");

    expect(invokeMock).toHaveBeenCalledWith("stop_session", {
      sessionId: "agent-1",
    });
  });
});

describe("onAgentStatus", () => {
  beforeEach(() => {
    listenMock.mockClear();
    clearHandlers();
  });

  it("calls listen with 'agent_status' and passes payload to callback", async () => {
    const cb = vi.fn();
    await onAgentStatus(cb);

    pushEvent({ agentId: "agent-1", status: "working" });

    expect(listenMock).toHaveBeenCalledWith("agent_status", expect.any(Function));
    expect(cb).toHaveBeenCalledWith({ agentId: "agent-1", status: "working" });
  });
});

describe("diff api wrappers", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("getDiff invokes get_diff with taskId", async () => {
    invokeMock.mockResolvedValueOnce("--- a/file.ts\n+++ b/file.ts\n");

    const result = await getDiff("task-1");

    expect(invokeMock).toHaveBeenCalledWith("get_diff", { taskId: "task-1", scope: "uncommitted" });
    expect(result).toBe("--- a/file.ts\n+++ b/file.ts\n");
  });

  it("getChangedFiles invokes get_changed_files with taskId", async () => {
    const files = [{ path: "src/foo.ts", change: "modified" }];
    invokeMock.mockResolvedValueOnce(files);

    const result = await getChangedFiles("task-1");

    expect(invokeMock).toHaveBeenCalledWith("get_changed_files", { taskId: "task-1", scope: "uncommitted" });
    expect(result).toEqual(files);
  });

  it("stageFiles invokes stage_files with taskId and paths", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await stageFiles("task-1", ["src/foo.ts", "src/bar.ts"]);

    expect(invokeMock).toHaveBeenCalledWith("stage_files", {
      taskId: "task-1",
      paths: ["src/foo.ts", "src/bar.ts"],
    });
  });

  it("commitTask invokes commit_task (not commit) with taskId and message", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await commitTask("task-1", "feat: add login");

    expect(invokeMock).toHaveBeenCalledWith("commit_task", {
      taskId: "task-1",
      message: "feat: add login",
    });
  });
});

describe("finishTask", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("finishTask invokes finish_task with taskId and mode 'keep'", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await finishTask("task-1", "keep");

    expect(invokeMock).toHaveBeenCalledWith("finish_task", {
      taskId: "task-1",
      mode: "keep",
    });
  });

  it("finishTask invokes finish_task with taskId and mode 'discard'", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await finishTask("task-1", "discard");

    expect(invokeMock).toHaveBeenCalledWith("finish_task", {
      taskId: "task-1",
      mode: "discard",
    });
  });

  it("finishTask invokes finish_task with taskId and mode 'merge'", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await finishTask("task-1", "merge");

    expect(invokeMock).toHaveBeenCalledWith("finish_task", {
      taskId: "task-1",
      mode: "merge",
    });
  });
});

describe("PR api wrappers", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    openUrlMock.mockReset();
  });

  it("ghStatus invokes gh_status with no args", async () => {
    invokeMock.mockResolvedValueOnce({ available: true, authenticated: true });

    const result = await ghStatus();

    expect(invokeMock).toHaveBeenCalledWith("gh_status");
    expect(result).toEqual({ available: true, authenticated: true });
  });

  it("createPr invokes create_pr with taskId, title, body, draft", async () => {
    invokeMock.mockResolvedValueOnce({ number: 42, url: "https://github.com/foo/bar/pull/42" });

    const result = await createPr("task-1", "My PR", "Description", false);

    expect(invokeMock).toHaveBeenCalledWith("create_pr", {
      taskId: "task-1",
      title: "My PR",
      body: "Description",
      draft: false,
    });
    expect(result).toEqual({ number: 42, url: "https://github.com/foo/bar/pull/42" });
  });

  it("createPr passes draft=true when draft flag is set", async () => {
    invokeMock.mockResolvedValueOnce({ number: 5, url: "https://github.com/foo/bar/pull/5" });

    await createPr("task-2", "Draft PR", "", true);

    expect(invokeMock).toHaveBeenCalledWith("create_pr", {
      taskId: "task-2",
      title: "Draft PR",
      body: "",
      draft: true,
    });
  });

  it("getPrStatus invokes get_pr_status with taskId", async () => {
    const status = {
      number: 1,
      url: "https://github.com/foo/bar/pull/1",
      title: "PR title",
      state: "open",
      isDraft: false,
      mergeable: "MERGEABLE",
      reviewDecision: null,
      checks: [],
    };
    invokeMock.mockResolvedValueOnce(status);

    const result = await getPrStatus("task-1");

    expect(invokeMock).toHaveBeenCalledWith("get_pr_status", { taskId: "task-1" });
    expect(result).toEqual(status);
  });

  it("getPrStatus returns null when no PR exists", async () => {
    invokeMock.mockResolvedValueOnce(null);

    const result = await getPrStatus("task-1");

    expect(invokeMock).toHaveBeenCalledWith("get_pr_status", { taskId: "task-1" });
    expect(result).toBeNull();
  });

  it("getPrComments invokes get_pr_comments with taskId", async () => {
    const comments = [
      { author: "alice", body: "LGTM", createdAt: "2024-01-01", path: null, line: null, kind: "issue_comment", state: null },
    ];
    invokeMock.mockResolvedValueOnce(comments);

    const result = await getPrComments("task-1");

    expect(invokeMock).toHaveBeenCalledWith("get_pr_comments", { taskId: "task-1" });
    expect(result).toEqual(comments);
  });

  it("openUrl delegates to plugin-opener openUrl", async () => {
    openUrlMock.mockResolvedValueOnce(undefined);

    await openUrl("https://github.com/foo/bar/pull/1");

    expect(openUrlMock).toHaveBeenCalledWith("https://github.com/foo/bar/pull/1");
  });
});

describe("setSoundSettings", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("invokes set_sound_settings with the json string", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await setSoundSettings('{"muted":true}');
    expect(invokeMock).toHaveBeenCalledWith("set_sound_settings", {
      settings: '{"muted":true}',
    });
  });
});

describe("agent management api wrappers", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("listAgents invokes list_agents", async () => {
    invokeMock.mockResolvedValue([]);
    await listAgents();
    expect(invokeMock).toHaveBeenCalledWith("list_agents");
  });

  it("setTaskAgent passes taskId + agent (null clears)", async () => {
    invokeMock.mockResolvedValue(undefined);
    await setTaskAgent("t1", "aider");
    expect(invokeMock).toHaveBeenCalledWith("set_task_agent", { taskId: "t1", agent: "aider" });
    await setTaskAgent("t1", null);
    expect(invokeMock).toHaveBeenCalledWith("set_task_agent", { taskId: "t1", agent: null });
  });

  it("upsert/deleteCustomAgent pass spec/name", async () => {
    invokeMock.mockResolvedValue(undefined);
    const spec = { name: "x", displayName: "X", binary: "x", baseArgs: [], resumeArgs: [], extraArgs: [], promptMode: "none", status: "lifecycle", builtin: false } as const;
    await upsertCustomAgent(spec);
    expect(invokeMock).toHaveBeenCalledWith("upsert_custom_agent", { spec });
    await deleteCustomAgent("x");
    expect(invokeMock).toHaveBeenCalledWith("delete_custom_agent", { name: "x" });
  });

  it("createTask forwards agent (null when omitted)", async () => {
    invokeMock.mockResolvedValue({ id: "t1" });
    await createTask("repo-1", "My task", undefined, undefined, "aider");
    expect(invokeMock).toHaveBeenCalledWith("create_task", expect.objectContaining({ agent: "aider" }));
    await createTask("repo-1", "My task");
    expect(invokeMock).toHaveBeenCalledWith("create_task", expect.objectContaining({ agent: null }));
  });
});

describe("task docs api wrappers", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("listTaskDocs invokes list_task_docs with camelCase taskId", async () => {
    invokeMock.mockResolvedValue([{ id: "memory/spec_TASK-54.md", label: "Spec (TASK-54)" }]);
    const docs = await listTaskDocs("task-1");
    expect(invokeMock).toHaveBeenCalledWith("list_task_docs", { taskId: "task-1" });
    expect(docs).toEqual([{ id: "memory/spec_TASK-54.md", label: "Spec (TASK-54)" }]);
  });

  it("readTaskDoc invokes read_task_doc with taskId and id", async () => {
    invokeMock.mockResolvedValue("# hello");
    const md = await readTaskDoc("task-1", "memory/spec_TASK-54.md");
    expect(invokeMock).toHaveBeenCalledWith("read_task_doc", {
      taskId: "task-1",
      id: "memory/spec_TASK-54.md",
    });
    expect(md).toBe("# hello");
  });
});

describe("model api", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("listAgentModels forwards agentName", async () => {
    invokeMock.mockResolvedValue(["zhipuai-coding-plan/glm-5.2"]);
    await expect(listAgentModels("opencode")).resolves.toEqual(["zhipuai-coding-plan/glm-5.2"]);
    expect(invokeMock).toHaveBeenCalledWith("list_agent_models", { agentName: "opencode" });
  });

  it("setTaskModel forwards taskId + model", async () => {
    invokeMock.mockResolvedValue(undefined);
    await setTaskModel("t1", "zhipuai-coding-plan/glm-5.2");
    expect(invokeMock).toHaveBeenCalledWith("set_task_model", { taskId: "t1", model: "zhipuai-coding-plan/glm-5.2" });
  });
});

describe("prompt api wrappers", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("createPrompt invokes create_prompt with label and body", async () => {
    invokeMock.mockResolvedValueOnce({ id: "x", label: "L", body: "B", position: 0 });
    const p = await createPrompt("L", "B");
    expect(invokeMock).toHaveBeenCalledWith("create_prompt", { label: "L", body: "B" });
    expect(p.id).toBe("x");
  });

  it("reorderPrompts invokes reorder_prompts with orderedIds", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await reorderPrompts(["b", "a"]);
    expect(invokeMock).toHaveBeenCalledWith("reorder_prompts", { orderedIds: ["b", "a"] });
  });
});

describe("notifyAgentEvent", () => {
  beforeEach(() => {
    isPermissionGrantedMock.mockReset();
    requestPermissionMock.mockReset();
    sendNotificationMock.mockReset();
    onActionMock.mockReset();
    onActionMock.mockResolvedValue(() => {});
  });

  it("permission already granted → notifies without requesting permission", async () => {
    isPermissionGrantedMock.mockResolvedValue(true);
    sendNotificationMock.mockResolvedValue(undefined);

    await notifyAgentEvent({ title: "T", body: "B", taskId: "t1" });

    expect(sendNotificationMock).toHaveBeenCalledOnce();
    expect(sendNotificationMock).toHaveBeenCalledWith({ id: expect.any(Number), title: "T", body: "B" });
    expect(requestPermissionMock).not.toHaveBeenCalled();
  });

  it("denied → does not notify", async () => {
    isPermissionGrantedMock.mockResolvedValue(false);
    requestPermissionMock.mockResolvedValue("denied");

    await notifyAgentEvent({ title: "T", body: "B", taskId: "t1" });

    expect(sendNotificationMock).not.toHaveBeenCalled();
  });

  it("not granted then user grants → notifies", async () => {
    isPermissionGrantedMock.mockResolvedValue(false);
    requestPermissionMock.mockResolvedValue("granted");
    sendNotificationMock.mockResolvedValue(undefined);

    await notifyAgentEvent({ title: "T", body: "B", taskId: "t1" });

    expect(requestPermissionMock).toHaveBeenCalledOnce();
    expect(sendNotificationMock).toHaveBeenCalledOnce();
    expect(sendNotificationMock).toHaveBeenCalledWith({ id: expect.any(Number), title: "T", body: "B" });
  });
});
