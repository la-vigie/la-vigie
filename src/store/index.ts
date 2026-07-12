import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { removeRepo as apiRemoveRepo, deleteTask as apiDeleteTask, stopSession, setSoundSettings as apiSetSoundSettings, setFetchRemoteBase as apiSetFetchRemoteBase, setInjectLavigieSkills as apiSetInjectLavigieSkills, listCustomSounds, enableRemote, disableRemote, remoteStatus, setTaskHidden, type RemoteStatus, listPrompts, createPrompt, updatePrompt, deletePrompt, reorderPrompts, type Prompt } from "../api";
import { DEFAULT_SOUND_SETTINGS, type SoundSettings, type CustomSound } from "../sound/types";
import { parseSoundSettings } from "../sound/safe-parse";

export type TaskStatus =
  | "idle"
  | "working"
  | "needs_attention"
  | "done"
  | "error"
  | "pending";

export type SetupStatus = "running" | "succeeded" | "failed";

export interface Repo {
  id: string;
  name: string;
  path: string;
  defaultBranch: string;
  remoteUrl?: string;
  worktreeRoot?: string | null;
  setupCommand?: string | null;
  defaultAgent?: string | null;
  autoStartAgent?: boolean;
  initialPrompt?: string | null;
  defaultModel?: string | null;
  soundSettings?: string | null;
  fetchRemoteBase?: boolean | null;
  autoApprove?: boolean | null;
}

export interface Task {
  id: string;
  repoId: string;
  title: string;
  worktreePath: string;
  branch: string;
  baseBranch: string;
  status: TaskStatus;
  createdAt: number;
  updatedAt: number;
  prNumber?: number | null;
  prUrl?: string | null;
  ticketKey?: string | null;
  agent?: string | null;
  model?: string | null;
  setupStatus?: SetupStatus | null;
  pendingPrompt?: string | null;
  hidden?: boolean;
  autoApprove?: boolean | null;
}

export type PromptMode = "stdin" | "arg" | "none";
export type StatusMechanism = "claudeHooks" | "lifecycle";

export interface AgentSpec {
  name: string;
  displayName: string;
  binary: string;
  baseArgs: readonly string[];
  resumeArgs: readonly string[];
  autoApproveArgs?: readonly string[];
  extraArgs: readonly string[];
  promptMode: PromptMode;
  status: StatusMechanism;
  builtin: boolean;
  modelArg?: string | null;
  modelsListArgs?: string[] | null;
}

interface AppSnapshot {
  repos: Repo[];
  tasks: Task[];
  worktreesRoot: string;
  soundSettings?: string | null;
  fetchRemoteBase?: boolean | null;
  injectLavigieSkills?: boolean | null;
}

export type AgentStatus = "starting" | "running" | "exited";

export type AgentActivity = "working" | "needs_attention" | "idle" | "error";

export const AGENT_TAB = "agent";
export type SessionKind = "agent" | "shell";
export interface TerminalSession {
  localId: string;
  kind: SessionKind;
  backendId?: string;
  status: AgentStatus;
  title: string;
  resume?: boolean;
  initialPrompt?: string;
  activity?: AgentActivity;
  lifecycle?: boolean;
}

export type Theme = "dark" | "light";

export interface ConsoleStatus {
  model?: string;
  contextRemainingPercent?: number;
  mode?: string;
}

export interface VigieState {
  repos: Repo[];
  tasks: Task[];
  worktreesRoot: string;
  selectedTaskId: string | null;
  sessionsByTask: Record<string, TerminalSession[]>;
  activeTabByTask: Record<string, string>;
  /** Per-task "needs your attention" flag, set when a non-selected task's
   *  agent finishes / needs input / errors, cleared when the task is viewed. */
  attentionByTask: Record<string, boolean>;
  consoleByAgentId: Record<string, ConsoleStatus>;
  setupByTask: Record<string, { status: SetupStatus; log: string; exitCode?: number | null; dismissed?: boolean }>;
  sidebarCollapsed: boolean;
  sidebarWidth: number;
  theme: Theme;
  soundSettings: SoundSettings;
  customSounds: CustomSound[];
  refreshCustomSounds: () => Promise<void>;
  prompts: Prompt[];
  refreshPrompts: () => Promise<void>;
  addPrompt: (label: string, body: string) => Promise<void>;
  editPrompt: (id: string, label: string, body: string) => Promise<void>;
  removePrompt: (id: string) => Promise<void>;
  movePrompt: (id: string, dir: "up" | "down") => Promise<void>;
  settingsOpen: boolean;
  openSettings: () => void;
  closeSettings: () => void;
  fetchRemoteBase: boolean;
  injectLavigieSkills: boolean;
  remote: RemoteStatus;
  toggleTheme: () => void;
  setSoundSettings: (next: SoundSettings) => Promise<void>;
  setFetchRemoteBase: (enabled: boolean) => Promise<void>;
  setInjectLavigieSkills: (enabled: boolean) => Promise<void>;
  setSelectedTask: (id: string | null) => void;
  setRepos: (repos: Repo[]) => void;
  setTasks: (tasks: Task[]) => void;
  /** Patch one task's title in place (TASK-40 agent-driven rename), leaving all
   *  other task state untouched. */
  setTaskTitle: (taskId: string, title: string) => void;
  refresh: () => Promise<void>;
  /** Remove a repo and its tasks: detaches in the backend (which also cleans up
   *  the worktrees La Vigie created), drops local session/selection state for
   *  the removed tasks, then refreshes. */
  removeRepo: (repoId: string) => Promise<void>;
  deleteTask: (taskId: string, deleteBranch: boolean) => Promise<void>;
  /** Handle a backend-emitted `task_removed` event (TASK-139): the backend
   *  already deleted the DB row (e.g. self-teardown via `/finish/{agentId}`),
   *  so this only reproduces the local half of `deleteTask` — clear per-task
   *  state, deselect if selected, and refresh. */
  handleTaskRemoved: (taskId: string) => Promise<void>;
  hideTask: (taskId: string) => Promise<void>;
  reopenTask: (taskId: string) => Promise<void>;
  startAgentSession: (taskId: string, resume: boolean, agent?: { label: string; lifecycle: boolean }, initialPrompt?: string) => void;
  removeAgentSession: (taskId: string) => void;
  addShellSession: (taskId: string) => void;
  removeShellSession: (taskId: string, localId: string) => void;
  setSessionInfo: (taskId: string, localId: string, partial: Partial<TerminalSession>) => void;
  setSessionActivity: (backendId: string, activity: AgentActivity) => void;
  setActiveTab: (taskId: string, localId: string) => void;
  clearTaskSessions: (taskId: string) => void;
  setAgentConsole: (agentId: string, partial: ConsoleStatus) => void;
  appendSetupOutput: (taskId: string, data: string) => void;
  setSetupStatus: (taskId: string, status: SetupStatus, exitCode?: number | null) => void;
  hydrateSetup: (taskId: string, status: SetupStatus, log: string, exitCode?: number | null) => void;
  /** Dismiss a task's setup strip (✕) — hides it for the rest of the session. */
  dismissSetup: (taskId: string) => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setSidebarWidth: (width: number) => void;
  refreshRemote: () => Promise<void>;
  enableRemoteControl: () => Promise<void>;
  disableRemoteControl: () => Promise<void>;
}

function initialTheme(): Theme {
  const stored = localStorage.getItem("vigie.theme");
  const theme: Theme = stored === "light" ? "light" : "dark";
  // Keep the document attribute in sync from the very first paint.
  document.documentElement.setAttribute("data-theme", theme);
  return theme;
}

export const useVigieStore = create<VigieState>((set, get) => ({
  repos: [],
  tasks: [],
  worktreesRoot: "",
  selectedTaskId: null,
  sessionsByTask: {},
  activeTabByTask: {},
  attentionByTask: {},
  consoleByAgentId: {},
  setupByTask: {},
  sidebarCollapsed: localStorage.getItem("vigie.sidebarCollapsed") === "true",
  sidebarWidth: Number(localStorage.getItem("vigie.sidebarWidth")) || 260,
  theme: initialTheme(),
  soundSettings: DEFAULT_SOUND_SETTINGS,
  customSounds: [],
  prompts: [],
  settingsOpen: false,
  fetchRemoteBase: true,
  injectLavigieSkills: false,
  remote: { active: false, sleepInhibited: false },
  toggleTheme: () =>
    set((state) => {
      const theme: Theme = state.theme === "dark" ? "light" : "dark";
      localStorage.setItem("vigie.theme", theme);
      document.documentElement.setAttribute("data-theme", theme);
      return { theme };
    }),
  setSelectedTask: (id) =>
    set((state) => {
      if (id == null) return { selectedTaskId: id };
      // Viewing a task clears its attention cue.
      const { [id]: _cleared, ...attentionByTask } = state.attentionByTask;
      return { selectedTaskId: id, attentionByTask };
    }),
  setRepos: (repos) => set({ repos }),
  setTasks: (tasks) => set({ tasks }),
  setTaskTitle: (taskId, title) =>
    set((state) => ({
      tasks: state.tasks.map((t) => (t.id === taskId ? { ...t, title } : t)),
    })),
  refresh: async () => {
    const snapshot = await invoke<AppSnapshot>("list_state");
    const parsed = parseSoundSettings(snapshot.soundSettings);
    set({
      repos: snapshot.repos,
      tasks: snapshot.tasks,
      worktreesRoot: snapshot.worktreesRoot,
      soundSettings: parsed
        ? {
            muted: parsed.muted ?? DEFAULT_SOUND_SETTINGS.muted,
            automute: parsed.automute ?? DEFAULT_SOUND_SETTINGS.automute,
            events: {
              completed: { ...DEFAULT_SOUND_SETTINGS.events.completed, ...(parsed.events?.completed ?? {}) },
              failed: { ...DEFAULT_SOUND_SETTINGS.events.failed, ...(parsed.events?.failed ?? {}) },
              awaitingInput: { ...DEFAULT_SOUND_SETTINGS.events.awaitingInput, ...(parsed.events?.awaitingInput ?? {}) },
            },
          }
        : DEFAULT_SOUND_SETTINGS,
      fetchRemoteBase: snapshot.fetchRemoteBase ?? true,
      injectLavigieSkills: snapshot.injectLavigieSkills ?? false,
    });
    await get().refreshCustomSounds();
    await get().refreshPrompts();
  },
  removeRepo: async (repoId) => {
    await apiRemoveRepo(repoId);
    const { tasks, selectedTaskId, clearTaskSessions, refresh } = get();
    const removedIds = tasks.filter((t) => t.repoId === repoId).map((t) => t.id);
    // Drop per-task session/attention/console state for the removed tasks.
    for (const id of removedIds) clearTaskSessions(id);
    // Clear the selection if it pointed at a removed task.
    if (selectedTaskId && removedIds.includes(selectedTaskId)) {
      set({ selectedTaskId: null });
    }
    await refresh();
  },
  deleteTask: async (taskId, deleteBranch) => {
    const { sessionsByTask, selectedTaskId, clearTaskSessions, refresh } = get();
    // Mirror Finish teardown so the selected task's TerminalHost dies cleanly
    // (KEEP-ALIVE): stop sessions, drop per-task state, then delete + refresh.
    const sessions = sessionsByTask[taskId] ?? [];
    await Promise.all(
      sessions.filter((s) => s.backendId).map((s) => stopSession(s.backendId!).catch(() => {})),
    );
    clearTaskSessions(taskId);
    await apiDeleteTask(taskId, deleteBranch);
    if (selectedTaskId === taskId) set({ selectedTaskId: null });
    await refresh();
  },
  handleTaskRemoved: async (taskId: string) => {
    const { selectedTaskId, clearTaskSessions, refresh } = get();
    clearTaskSessions(taskId);
    if (selectedTaskId === taskId) set({ selectedTaskId: null });
    await refresh();
  },
  hideTask: async (taskId) => {
    await setTaskHidden(taskId, true);
    await get().refresh();
  },
  reopenTask: async (taskId) => {
    await setTaskHidden(taskId, false);
    await get().refresh();
  },
  startAgentSession: (taskId, resume, agent, initialPrompt) =>
    set((state) => {
      const rest = (state.sessionsByTask[taskId] ?? []).filter((s) => s.kind !== "agent");
      const agentSession: TerminalSession = {
        localId: AGENT_TAB,
        kind: "agent",
        status: "starting",
        title: agent?.label ?? "Claude",
        lifecycle: agent?.lifecycle ?? false,
        resume,
        initialPrompt,
      };
      return {
        sessionsByTask: { ...state.sessionsByTask, [taskId]: [agentSession, ...rest] },
        activeTabByTask: { ...state.activeTabByTask, [taskId]: AGENT_TAB },
      };
    }),
  removeAgentSession: (taskId) =>
    set((state) => {
      const prev = state.sessionsByTask[taskId] ?? [];
      const agentBackendId = prev.find((s) => s.kind === "agent")?.backendId;
      const sessions = prev.filter((s) => s.kind !== "agent");
      const consoleByAgentId = { ...state.consoleByAgentId };
      if (agentBackendId) delete consoleByAgentId[agentBackendId];
      return {
        sessionsByTask: { ...state.sessionsByTask, [taskId]: sessions },
        consoleByAgentId,
      };
    }),
  addShellSession: (taskId) =>
    set((state) => {
      const sessions = state.sessionsByTask[taskId] ?? [];
      const shellCount = sessions.filter((s) => s.kind === "shell").length;
      const title = shellCount === 0 ? "shell" : `shell ${shellCount + 1}`;
      const localId = crypto.randomUUID();
      const shell: TerminalSession = { localId, kind: "shell", status: "starting", title };
      return {
        sessionsByTask: { ...state.sessionsByTask, [taskId]: [...sessions, shell] },
        activeTabByTask: { ...state.activeTabByTask, [taskId]: localId },
      };
    }),
  removeShellSession: (taskId, localId) =>
    set((state) => {
      const prev = state.sessionsByTask[taskId] ?? [];
      const idx = prev.findIndex((s) => s.localId === localId);
      const sessions = prev.filter((s) => s.localId !== localId);
      let active = state.activeTabByTask[taskId];
      if (active === localId) {
        const neighbor = sessions[idx] ?? sessions[idx - 1] ?? sessions[0];
        active = neighbor?.localId ?? AGENT_TAB;
      }
      return {
        sessionsByTask: { ...state.sessionsByTask, [taskId]: sessions },
        activeTabByTask: { ...state.activeTabByTask, [taskId]: active },
      };
    }),
  setSessionInfo: (taskId, localId, partial) =>
    set((state) => {
      const sessions = state.sessionsByTask[taskId];
      if (!sessions) return state;
      return {
        sessionsByTask: {
          ...state.sessionsByTask,
          [taskId]: sessions.map((s) => (s.localId === localId ? { ...s, ...partial } : s)),
        },
      };
    }),
  setSessionActivity: (backendId, activity) =>
    set((state) => {
      let foundTask: string | undefined;
      const sessionsByTask = Object.fromEntries(
        Object.entries(state.sessionsByTask).map(([taskId, sessions]) => [
          taskId,
          sessions.map((s) => {
            if (s.kind === "agent" && s.backendId === backendId) {
              foundTask = taskId;
              return { ...s, activity };
            }
            return s;
          }),
        ]),
      );
      if (!foundTask) return state;
      const attentionWorthy = activity === "needs_attention" || activity === "idle" || activity === "error";
      if (attentionWorthy && foundTask !== state.selectedTaskId) {
        return { sessionsByTask, attentionByTask: { ...state.attentionByTask, [foundTask]: true } };
      }
      return { sessionsByTask };
    }),
  setActiveTab: (taskId, localId) =>
    set((state) => ({ activeTabByTask: { ...state.activeTabByTask, [taskId]: localId } })),
  clearTaskSessions: (taskId) =>
    set((state) => {
      const { [taskId]: removed, ...sessionsByTask } = state.sessionsByTask;
      const { [taskId]: _a, ...attentionByTask } = state.attentionByTask;
      const { [taskId]: _t, ...activeTabByTask } = state.activeTabByTask;
      const { [taskId]: _s, ...setupByTask } = state.setupByTask;
      const consoleByAgentId = { ...state.consoleByAgentId };
      for (const s of removed ?? []) if (s.backendId) delete consoleByAgentId[s.backendId];
      return { sessionsByTask, attentionByTask, activeTabByTask, consoleByAgentId, setupByTask };
    }),
  appendSetupOutput: (taskId, data) =>
    set((state) => {
      const prev = state.setupByTask[taskId] ?? { status: "running" as SetupStatus, log: "" };
      return { setupByTask: { ...state.setupByTask, [taskId]: { ...prev, log: prev.log + data } } };
    }),
  setSetupStatus: (taskId, status, exitCode) =>
    set((state) => {
      const prev = state.setupByTask[taskId] ?? { status, log: "" };
      return { setupByTask: { ...state.setupByTask, [taskId]: { ...prev, status, exitCode } } };
    }),
  hydrateSetup: (taskId, status, log, exitCode) =>
    set((state) => ({
      // Preserve a prior dismissal so re-opening a task doesn't resurrect a
      // strip the user already closed this session.
      setupByTask: {
        ...state.setupByTask,
        [taskId]: { status, log, exitCode, dismissed: state.setupByTask[taskId]?.dismissed },
      },
    })),
  dismissSetup: (taskId) =>
    set((state) => {
      const prev = state.setupByTask[taskId];
      if (!prev) return state;
      return { setupByTask: { ...state.setupByTask, [taskId]: { ...prev, dismissed: true } } };
    }),
  setAgentConsole: (agentId, partial) =>
    set((state) => ({
      consoleByAgentId: {
        ...state.consoleByAgentId,
        [agentId]: { ...state.consoleByAgentId[agentId], ...partial },
      },
    })),
  setSoundSettings: async (next) => {
    set({ soundSettings: next });
    try {
      await apiSetSoundSettings(JSON.stringify(next));
    } catch (err) {
      console.error("Failed to persist sound settings", err);
    }
  },
  refreshCustomSounds: async () => {
    try {
      set({ customSounds: await listCustomSounds() });
    } catch (err) {
      console.error("Failed to load custom sounds", err);
    }
  },
  refreshPrompts: async () => {
    try {
      set({ prompts: await listPrompts() });
    } catch (err) {
      console.error("Failed to load prompts", err);
    }
  },
  addPrompt: async (label, body) => {
    await createPrompt(label, body);
    await get().refreshPrompts();
  },
  editPrompt: async (id, label, body) => {
    await updatePrompt(id, label, body);
    await get().refreshPrompts();
  },
  removePrompt: async (id) => {
    await deletePrompt(id);
    await get().refreshPrompts();
  },
  movePrompt: async (id, dir) => {
    const ids = get().prompts.map((p) => p.id);
    const i = ids.indexOf(id);
    const j = dir === "up" ? i - 1 : i + 1;
    if (i < 0 || j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    await reorderPrompts(ids);
    await get().refreshPrompts();
  },
  openSettings: () => set({ settingsOpen: true }),
  closeSettings: () => set({ settingsOpen: false }),
  setFetchRemoteBase: async (enabled) => {
    set({ fetchRemoteBase: enabled });
    try {
      await apiSetFetchRemoteBase(enabled);
    } catch (err) {
      console.error("Failed to persist fetch-remote-base setting", err);
    }
  },
  setInjectLavigieSkills: async (enabled) => {
    set({ injectLavigieSkills: enabled });
    try {
      await apiSetInjectLavigieSkills(enabled);
    } catch (err) {
      console.error("Failed to persist inject-lavigie-skills setting", err);
    }
  },
  setSidebarCollapsed: (collapsed) => {
    localStorage.setItem("vigie.sidebarCollapsed", String(collapsed));
    set({ sidebarCollapsed: collapsed });
  },
  setSidebarWidth: (width) => {
    localStorage.setItem("vigie.sidebarWidth", String(width));
    set({ sidebarWidth: width });
  },
  refreshRemote: async () => {
    set({ remote: await remoteStatus() });
  },
  enableRemoteControl: async () => {
    set({ remote: await enableRemote() });
  },
  disableRemoteControl: async () => {
    set({ remote: await disableRemote() });
  },
}));
