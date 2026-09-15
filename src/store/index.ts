import { Channel, invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { removeRepo as apiRemoveRepo, deleteTask as apiDeleteTask, finishTask as apiFinishTask, stopSession, setSoundSettings as apiSetSoundSettings, setFetchRemoteBase as apiSetFetchRemoteBase, setInjectLavigieSkills as apiSetInjectLavigieSkills, listCustomSounds, enableRemote, disableRemote, remoteStatus, setTaskHidden, type RemoteStatus, listPrompts, createPrompt, updatePrompt, deletePrompt, reorderPrompts, type Prompt, listAgents, startAcpAgent, acpPrompt, acpCancel, acpRespondPermission, acpSetMode, type AcpEvent } from "../api";
import {
  appendLocalUserMessage,
  clearPermission,
  emptyTimeline,
  reduceAcpEvent,
  type AcpTimelineState,
} from "../acp/timeline";
import { DEFAULT_SOUND_SETTINGS, type SoundSettings, type CustomSound } from "../sound/types";
import { parseSoundSettings } from "../sound/safe-parse";
import {
  createTour,
  startTour,
  next as tourNext,
  prev as tourPrev,
  skip as tourSkip,
  type TourState,
} from "../tour/tourMachine";
import { TOUR_STEPS } from "../tour/steps";
import { loadOnboarding, saveOnboarding, type PersistedStatus } from "../tour/persistence";

export type TaskStatus =
  | "idle"
  | "working"
  | "needs_attention"
  | "done"
  | "error"
  | "pending";

export type SetupStatus = "running" | "succeeded" | "failed";

export interface Blocker {
  taskId: string;
  title?: string | null;
  status?: TaskStatus | null;
}

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
  inPlaceDefault: boolean;
  /** JSON-encoded auto-routing policy, or null when routing is off. */
  routingPolicy?: string | null;
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
  /** Outstanding blockers this task is queued behind (pending tasks only). */
  blockedBy?: Blocker[];
  inPlace: boolean;
  /** Why the engine was auto-selected by routing (null if explicit/off). */
  routingReason?: string | null;
  /** TASK-252: the ACP session id this task last ran with (null ⇒ never ran an
   *  ACP agent). Gates the desktop Resume affordance for ACP engines — resume
   *  needs a stored session to reconnect to. */
  acpSessionId?: string | null;
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
  skillInjection?: "none" | "pluginDir" | { worktreeBundle: { provider: string } };
  /** Which backend runs this agent. The Rust side always serializes it
   *  ("pty" | "acp"); optional here so older fixtures/tests decode. */
  execution?: "pty" | "acp";
}

interface AppSnapshot {
  repos: Repo[];
  tasks: Task[];
  worktreesRoot: string;
  soundSettings?: string | null;
  fetchRemoteBase?: boolean | null;
  injectLavigieSkills?: boolean | null;
  blockedBy?: Record<string, Blocker[]>;
}

export type AgentStatus = "starting" | "running" | "exited";

export type AgentActivity = "working" | "needs_attention" | "idle" | "error";

export const AGENT_TAB = "agent";
export type SessionKind = "agent" | "shell" | "orchestrator";

/** Surface-id namespace for the worktree-less orchestrator chat. A
 *  "surface id" keys `sessionsByTask`/`activeTabByTask`/the TerminalHost and is
 *  either a task id or `orchestrator:{repoId}` — the prefix guarantees it can't
 *  collide with a task UUID. */
export const ORCHESTRATOR_PREFIX = "orchestrator:";
export function orchestratorSurfaceId(repoId: string): string {
  return `${ORCHESTRATOR_PREFIX}${repoId}`;
}
export function repoIdFromSurface(surfaceId: string): string {
  return surfaceId.startsWith(ORCHESTRATOR_PREFIX)
    ? surfaceId.slice(ORCHESTRATOR_PREFIX.length)
    : surfaceId;
}

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
  /** Which backend runs this session. Absent ⇒ "pty". ACP agent sessions get
   *  no TerminalView/PTY — they render the AcpSurface instead, and their live
   *  Channel is owned by the store, not a component. */
  engine?: "pty" | "acp";
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
  /** The repo whose worktree-less orchestrator chat is selected. Mutually
   *  exclusive with `selectedTaskId`. */
  selectedOrchestratorRepoId: string | null;
  sessionsByTask: Record<string, TerminalSession[]>;
  activeTabByTask: Record<string, string>;
  /** Per-task "needs your attention" flag, set when a non-selected task's
   *  agent finishes / needs input / errors, cleared when the task is viewed. */
  attentionByTask: Record<string, boolean>;
  consoleByAgentId: Record<string, ConsoleStatus>;
  /** Per-task last agent error text (from a StopFailure hook). Present ⇒ the
   *  TaskDetail error banner shows; cleared when the agent moves back to
   *  Working/Idle or the task's sessions are torn down. */
  errorByTask: Record<string, string>;
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
  // The ACP usage/cost panel overlay.
  usageOpen: boolean;
  openUsage: () => void;
  closeUsage: () => void;
  fetchRemoteBase: boolean;
  injectLavigieSkills: boolean;
  remote: RemoteStatus;
  toggleTheme: () => void;
  setSoundSettings: (next: SoundSettings) => Promise<void>;
  setFetchRemoteBase: (enabled: boolean) => Promise<void>;
  setInjectLavigieSkills: (enabled: boolean) => Promise<void>;
  setSelectedTask: (id: string | null) => void;
  setSelectedOrchestrator: (repoId: string | null) => void;
  startOrchestratorSession: (repoId: string) => void;
  removeOrchestratorSession: (repoId: string) => void;
  setRepos: (repos: Repo[]) => void;
  setTasks: (tasks: Task[]) => void;
  /** Patch one task's title in place (agent-driven rename), leaving all other
   *  task state untouched. */
  setTaskTitle: (taskId: string, title: string) => void;
  refresh: () => Promise<void>;
  /** Per-task nonce for the git/fs Review group (Diff + Spec + count badges).
   *  Bumping asks the currently-viewed ReviewPanel to re-fetch those. */
  reviewNonceByTask: Record<string, number>;
  /** Per-task nonce for the gh Review group (PR dock). Separate so it can carry
   *  a longer debounce window than the git/fs group. */
  prNonceByTask: Record<string, number>;
  /** Lightweight snapshot refresh: re-run list_state → repos/tasks/flags only
   *  (no prompt/custom-sound reloads). Safe to fire on frequent auto-triggers. */
  refreshSnapshot: () => Promise<void>;
  bumpReview: (taskId: string) => void;
  bumpPr: (taskId: string) => void;
  /** Remove a repo and its tasks: detaches in the backend (which also cleans up
   *  the worktrees La Vigie created), drops local session/selection state for
   *  the removed tasks, then refreshes. */
  removeRepo: (repoId: string) => Promise<void>;
  deleteTask: (taskId: string, deleteBranch: boolean) => Promise<void>;
  /** Finish a task: the shared teardown behind both the TaskDetail header
   *  "Finish task" button and the Sidebar right-click "Finish…" item.
   *  Mirrors `deleteTask`'s KEEP-ALIVE teardown so the selected task's
   *  TerminalHost dies cleanly: stop all sessions → drop per-task state →
   *  `finish_task` → deselect if it was selected → refresh. Rejects on backend
   *  failure so callers can surface the error and keep the task selected. */
  finishTask: (taskId: string, mode: "keep" | "discard" | "merge") => Promise<void>;
  /** Handle a backend-emitted `task_removed` event: the backend already
   *  deleted the DB row (e.g. self-teardown via `/finish/{agentId}`),
   *  so this only reproduces the local half of `deleteTask` — clear per-task
   *  state, deselect if selected, and refresh. */
  handleTaskRemoved: (taskId: string) => Promise<void>;
  hideTask: (taskId: string) => Promise<void>;
  reopenTask: (taskId: string) => Promise<void>;
  // Async: awaits the agent catalog before PTY/ACP routing when
  // it isn't loaded yet. Callers fire-and-forget; awaitable if you need the
  // session to exist (catalog-loaded-then-routed) before continuing.
  startAgentSession: (taskId: string, resume: boolean, agent?: { label: string; lifecycle: boolean }, initialPrompt?: string) => Promise<void>;
  removeAgentSession: (taskId: string) => void;

  // --- Agent spec catalog. Loaded once per app run (deduped in-flight),
  //     shared by useAgents() consumers and engine routing. ---
  agents: AgentSpec[];
  agentsLoaded: boolean;
  agentsError: string | null;
  loadAgents: () => Promise<void>;

  // --- ACP bubbles surface. One timeline per task, reduced from the
  //     session's `Channel<AcpEvent>` (owned by the store — the surface
  //     component is a pure view and may unmount freely). ---
  acpByTask: Record<string, AcpTimelineState>;
  /** Fold one backend event into a task's timeline (exported for tests). */
  applyAcpEvent: (taskId: string, event: AcpEvent) => void;
  /** Send a follow-up prompt on the task's live ACP session (optimistic user
   *  bubble + `acp_prompt`). Rejects when no live session id exists yet. */
  sendAcpPrompt: (taskId: string, text: string) => Promise<void>;
  /** Cancel the in-flight turn on the task's live ACP session. */
  cancelAcpTurn: (taskId: string) => Promise<void>;
  /** Answer the pending permission request (omitted optionId = cancel). */
  respondAcpPermission: (taskId: string, requestId: string, optionId?: string) => Promise<void>;
  /** Switch the session's mode (optimistic; `modeChanged` confirms). */
  setAcpMode: (taskId: string, modeId: string) => Promise<void>;
  addShellSession: (taskId: string) => void;
  removeShellSession: (taskId: string, localId: string) => void;
  setSessionInfo: (taskId: string, localId: string, partial: Partial<TerminalSession>) => void;
  setSessionActivity: (backendId: string, activity: AgentActivity) => void;
  /** Store (message) or clear (null) a task's last agent error text. */
  setTaskError: (taskId: string, message: string | null) => void;
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

  // --- First-run onboarding tour (pure machine in src/tour, persisted to
  //     localStorage). Actions delegate to the reducers, then persist. ---
  onboarding: TourState;
  /** Persisted "has this install seen the tour" status, read at init. */
  onboardingStatus: PersistedStatus;
  startOnboarding: () => void;
  onboardingNext: () => void;
  onboardingPrev: () => void;
  skipOnboarding: () => void;
}

function initialTheme(): Theme {
  const stored = localStorage.getItem("vigie.theme");
  const theme: Theme = stored === "light" ? "light" : "dark";
  // Keep the document attribute in sync from the very first paint.
  document.documentElement.setAttribute("data-theme", theme);
  return theme;
}

// Build the initial onboarding machine from persisted status. An interrupted
// ("active") tour resumes at its saved step id; anything else stays idle (the
// <Tour/> overlay auto-starts a "pending" tour once, after mount).
function initialOnboarding(): { onboarding: TourState; onboardingStatus: PersistedStatus } {
  const persisted = loadOnboarding();
  const base = createTour(TOUR_STEPS);
  const onboarding = persisted.status === "active" ? startTour(base, persisted.stepId) : base;
  return { onboarding, onboardingStatus: persisted.status };
}

// In-flight dedup for loadAgents (several consumers may mount at once).
let agentsInFlight: Promise<void> | null = null;

/** The task's live ACP session id, if the agent session has registered one. */
function acpBackendId(state: VigieState, taskId: string): string | undefined {
  return state.sessionsByTask[taskId]?.find((s) => s.kind === "agent" && s.engine === "acp")
    ?.backendId;
}

/** Spawn the ACP session for `taskId`. Module-level on purpose: the store —
 *  not a component — owns the Channel, so no unmount can kill a live session
 *  (the PTY KEEP-ALIVE risk class is designed out for ACP). */
function spawnAcpSession(taskId: string, resume: boolean, initialPrompt?: string): void {
  const channel = new Channel<AcpEvent>();
  channel.onmessage = (event) => {
    const store = useVigieStore.getState();
    store.applyAcpEvent(taskId, event);
    if (event.type === "exit") {
      // Mirror the PTY exit path: drop the session so the surface reverts to
      // the Start placeholder; best-effort backend cleanup.
      const backendId = store.sessionsByTask[taskId]?.find((s) => s.kind === "agent")?.backendId;
      store.removeAgentSession(taskId);
      if (backendId) stopSession(backendId).catch(() => {});
    }
  };
  startAcpAgent(taskId, resume, channel, initialPrompt)
    .then((id) => {
      const store = useVigieStore.getState();
      const session = store.sessionsByTask[taskId]?.find((s) => s.kind === "agent");
      if (!session || session.engine !== "acp") {
        // Torn down (or replaced) while the spawn was in flight.
        stopSession(id).catch(() => {});
        return;
      }
      store.setSessionInfo(taskId, AGENT_TAB, { backendId: id, status: "running" });
    })
    .catch((err) => {
      // Order matters: removeAgentSession clears errorByTask for the task, so
      // drop the session first, then surface why the spawn failed.
      const store = useVigieStore.getState();
      store.removeAgentSession(taskId);
      store.setTaskError(taskId, err instanceof Error ? err.message : String(err));
    });
}

export const useVigieStore = create<VigieState>((set, get) => ({
  repos: [],
  tasks: [],
  worktreesRoot: "",
  selectedTaskId: null,
  selectedOrchestratorRepoId: null,
  sessionsByTask: {},
  activeTabByTask: {},
  attentionByTask: {},
  consoleByAgentId: {},
  errorByTask: {},
  setupByTask: {},
  agents: [],
  agentsLoaded: false,
  agentsError: null,
  acpByTask: {},
  reviewNonceByTask: {},
  prNonceByTask: {},
  sidebarCollapsed: localStorage.getItem("vigie.sidebarCollapsed") === "true",
  sidebarWidth: Number(localStorage.getItem("vigie.sidebarWidth")) || 260,
  theme: initialTheme(),
  soundSettings: DEFAULT_SOUND_SETTINGS,
  customSounds: [],
  prompts: [],
  settingsOpen: false,
  usageOpen: false,
  fetchRemoteBase: true,
  injectLavigieSkills: false,
  remote: { active: false, sleepInhibited: false },
  ...initialOnboarding(),
  startOnboarding: () =>
    set((state) => {
      const onboarding = startTour(state.onboarding);
      saveOnboarding(onboarding);
      return { onboarding, onboardingStatus: "active" as PersistedStatus };
    }),
  onboardingNext: () =>
    set((state) => {
      const onboarding = tourNext(state.onboarding);
      saveOnboarding(onboarding);
      return { onboarding, onboardingStatus: onboarding.status as PersistedStatus };
    }),
  onboardingPrev: () =>
    set((state) => {
      const onboarding = tourPrev(state.onboarding);
      saveOnboarding(onboarding);
      return { onboarding };
    }),
  skipOnboarding: () =>
    set((state) => {
      const onboarding = tourSkip(state.onboarding);
      saveOnboarding(onboarding);
      return { onboarding, onboardingStatus: "dismissed" as PersistedStatus };
    }),
  toggleTheme: () =>
    set((state) => {
      const theme: Theme = state.theme === "dark" ? "light" : "dark";
      localStorage.setItem("vigie.theme", theme);
      document.documentElement.setAttribute("data-theme", theme);
      return { theme };
    }),
  setSelectedTask: (id) =>
    set((state) => {
      if (id == null) return { selectedTaskId: id, selectedOrchestratorRepoId: null };
      // Viewing a task clears its attention cue and any orchestrator selection.
      const { [id]: _cleared, ...attentionByTask } = state.attentionByTask;
      return { selectedTaskId: id, selectedOrchestratorRepoId: null, attentionByTask };
    }),
  setSelectedOrchestrator: (repoId) =>
    set({ selectedOrchestratorRepoId: repoId, selectedTaskId: null }),
  startOrchestratorSession: (repoId) =>
    set((state) => {
      const key = orchestratorSurfaceId(repoId);
      // Idempotent: if the orchestrator terminal is already open, keep the live
      // one (KEEP-ALIVE) — don't remount/respawn.
      if (state.sessionsByTask[key]?.some((s) => s.kind === "orchestrator")) return state;
      const session: TerminalSession = {
        localId: AGENT_TAB,
        kind: "orchestrator",
        status: "starting",
        title: "Orchestrator",
      };
      return {
        sessionsByTask: { ...state.sessionsByTask, [key]: [session] },
        activeTabByTask: { ...state.activeTabByTask, [key]: AGENT_TAB },
      };
    }),
  removeOrchestratorSession: (repoId) =>
    set((state) => {
      const key = orchestratorSurfaceId(repoId);
      const prev = state.sessionsByTask[key] ?? [];
      const backendId = prev.find((s) => s.kind === "orchestrator")?.backendId;
      const consoleByAgentId = { ...state.consoleByAgentId };
      if (backendId) delete consoleByAgentId[backendId];
      return {
        sessionsByTask: { ...state.sessionsByTask, [key]: prev.filter((s) => s.kind !== "orchestrator") },
        consoleByAgentId,
      };
    }),
  setRepos: (repos) => set({ repos }),
  setTasks: (tasks) => set({ tasks }),
  setTaskTitle: (taskId, title) =>
    set((state) => ({
      tasks: state.tasks.map((t) => (t.id === taskId ? { ...t, title } : t)),
    })),
  refresh: async () => {
    // Swallow+log on failure, mirroring refreshPrompts/refreshCustomSounds. Several
    // components fire refresh() fire-and-forget (Sidebar/SettingsModal/DiffPanel/
    // useTaskCreated); if it rejected, that un-awaited rejection would surface as a global
    // unhandled rejection and, under vitest's file-parallel/shuffled ordering, get
    // misattributed to an unrelated test. list_state does not fail in prod, so this
    // only ever guards test/edge conditions.
    try {
      const snapshot = await invoke<AppSnapshot>("list_state");
      const parsed = parseSoundSettings(snapshot.soundSettings);
      set({
        repos: snapshot.repos,
        tasks: snapshot.tasks.map((t) => ({
          ...t,
          blockedBy: snapshot.blockedBy?.[t.id] ?? [],
        })),
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
    } catch (err) {
      console.error("Failed to refresh state", err);
    }
  },
  refreshSnapshot: async () => {
    const snapshot = await invoke<AppSnapshot>("list_state");
    set({
      repos: snapshot.repos,
      tasks: snapshot.tasks,
      worktreesRoot: snapshot.worktreesRoot,
      fetchRemoteBase: snapshot.fetchRemoteBase ?? true,
      injectLavigieSkills: snapshot.injectLavigieSkills ?? false,
    });
  },
  bumpReview: (taskId) =>
    set((state) => ({
      reviewNonceByTask: {
        ...state.reviewNonceByTask,
        [taskId]: (state.reviewNonceByTask[taskId] ?? 0) + 1,
      },
    })),
  bumpPr: (taskId) =>
    set((state) => ({
      prNonceByTask: {
        ...state.prNonceByTask,
        [taskId]: (state.prNonceByTask[taskId] ?? 0) + 1,
      },
    })),
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
    // Clear the orchestrator selection if it pointed at the removed repo.
    // The backend revoke_orchestrator_for_repo already stops that repo's live
    // orchestrator; this drops the now-dangling desktop selection.
    if (get().selectedOrchestratorRepoId === repoId) {
      set({ selectedOrchestratorRepoId: null });
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
  finishTask: async (taskId, mode) => {
    const { sessionsByTask, selectedTaskId, clearTaskSessions, refresh } = get();
    // Same KEEP-ALIVE teardown as deleteTask: stop every backend session for
    // the task, drop its local session/tab state, THEN call finish_task. Any
    // finish_task rejection propagates (we haven't cleared selection yet), so
    // the caller can show the error without stranding the task unselected.
    const sessions = sessionsByTask[taskId] ?? [];
    await Promise.all(
      sessions.filter((s) => s.backendId).map((s) => stopSession(s.backendId!).catch(() => {})),
    );
    clearTaskSessions(taskId);
    await apiFinishTask(taskId, mode);
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
  startAgentSession: async (taskId, resume, agent, initialPrompt) => {
    // Resolve the task's effective agent spec (task ?? repo ?? "claude",
    // mirroring the backend) to route PTY vs ACP.
    //
    // Routing MUST see the catalog — a not-yet-loaded one makes an ACP task
    // fall through to PTY, and the backend `start_agent` command then rejects
    // it ("'claude-acp' is an ACP agent; use start_acp_agent"). The catalog is
    // loaded eagerly at app boot (App.tsx) and on every picker mount, but a
    // launch that fires before that resolves would still misroute — so ensure
    // it here before deciding. (loadAgents dedups an in-flight load, so this is
    // at most one shared round-trip, and a no-op once loaded.)
    if (!get().agentsLoaded) await get().loadAgents();
    const { tasks, repos, agents } = get();
    const task = tasks.find((t) => t.id === taskId);
    const repo = task ? repos.find((r) => r.id === task.repoId) : undefined;
    const agentName = task?.agent ?? repo?.defaultAgent ?? "claude";
    const spec = agents.find((a) => a.name === agentName);
    const engine: "pty" | "acp" = spec?.execution === "acp" ? "acp" : "pty";
    set((state) => {
      const rest = (state.sessionsByTask[taskId] ?? []).filter((s) => s.kind !== "agent");
      const agentSession: TerminalSession = {
        localId: AGENT_TAB,
        kind: "agent",
        status: "starting",
        title: agent?.label ?? spec?.displayName ?? "Claude",
        lifecycle: agent?.lifecycle ?? (spec ? spec.status === "lifecycle" : false),
        resume,
        initialPrompt,
        engine,
      };
      // A fresh agent run starts from a fresh timeline (mirrors the PTY
      // surface, whose scrollback dies with the previous TerminalView).
      const { [taskId]: _stale, ...acpRest } = state.acpByTask;
      return {
        sessionsByTask: { ...state.sessionsByTask, [taskId]: [agentSession, ...rest] },
        activeTabByTask: { ...state.activeTabByTask, [taskId]: AGENT_TAB },
        acpByTask: engine === "acp" ? { ...acpRest, [taskId]: emptyTimeline() } : state.acpByTask,
      };
    });
    // The PTY path spawns on TerminalView mount (KEEP-ALIVE); the ACP session
    // is spawned right here, store-owned — no component ever holds it.
    if (engine === "acp") spawnAcpSession(taskId, resume, initialPrompt);
  },

  loadAgents: async () => {
    if (agentsInFlight) return agentsInFlight;
    agentsInFlight = (async () => {
      try {
        const agents = await listAgents();
        // `?? []` keeps routing's `agents.find` safe if the invoke resolves
        // to a nullish value (e.g. an under-specified test mock).
        set({ agents: agents ?? [], agentsLoaded: true, agentsError: null });
      } catch (e) {
        console.error("listAgents failed:", e);
        // Deliberately DON'T flip `agentsLoaded` on failure: a first-load
        // failure leaves it false so the next launch/mount retries (a transient
        // failure self-heals), and startAgentSession's `!agentsLoaded` guard
        // won't route blind on an empty catalog. A failure after a prior
        // success leaves `agentsLoaded`/`agents` untouched, keeping the last
        // good catalog. (`useAgents` treats a set error as not-loading so the
        // UI doesn't spin.)
        set({ agentsError: String(e) });
      } finally {
        agentsInFlight = null;
      }
    })();
    return agentsInFlight;
  },

  applyAcpEvent: (taskId, event) =>
    set((state) => ({
      acpByTask: {
        ...state.acpByTask,
        [taskId]: reduceAcpEvent(state.acpByTask[taskId] ?? emptyTimeline(), event),
      },
    })),
  sendAcpPrompt: async (taskId, text) => {
    const backendId = acpBackendId(get(), taskId);
    if (!backendId) throw new Error("ACP session not ready");
    set((state) => ({
      acpByTask: {
        ...state.acpByTask,
        [taskId]: appendLocalUserMessage(state.acpByTask[taskId] ?? emptyTimeline(), text),
      },
    }));
    await acpPrompt(backendId, text);
  },
  cancelAcpTurn: async (taskId) => {
    const backendId = acpBackendId(get(), taskId);
    if (backendId) await acpCancel(backendId);
  },
  respondAcpPermission: async (taskId, requestId, optionId) => {
    const backendId = acpBackendId(get(), taskId);
    if (!backendId) throw new Error("ACP session not ready");
    await acpRespondPermission(backendId, requestId, optionId);
    set((state) => {
      const t = state.acpByTask[taskId];
      return t ? { acpByTask: { ...state.acpByTask, [taskId]: clearPermission(t) } } : state;
    });
  },
  setAcpMode: async (taskId, modeId) => {
    const backendId = acpBackendId(get(), taskId);
    if (!backendId) throw new Error("ACP session not ready");
    await acpSetMode(backendId, modeId);
    // Optimistic: the backend's `modeChanged` event confirms (idempotent).
    set((state) => {
      const t = state.acpByTask[taskId];
      if (!t) return state;
      return {
        acpByTask: { ...state.acpByTask, [taskId]: reduceAcpEvent(t, { type: "modeChanged", modeId }) },
      };
    });
  },
  removeAgentSession: (taskId) =>
    set((state) => {
      const prev = state.sessionsByTask[taskId] ?? [];
      const agentBackendId = prev.find((s) => s.kind === "agent")?.backendId;
      const sessions = prev.filter((s) => s.kind !== "agent");
      const consoleByAgentId = { ...state.consoleByAgentId };
      if (agentBackendId) delete consoleByAgentId[agentBackendId];
      // Drop the stale error banner when the agent session is removed.
      const { [taskId]: _e, ...errorByTask } = state.errorByTask;
      // The ACP timeline dies with its session (mirrors PTY scrollback).
      const { [taskId]: _acp, ...acpByTask } = state.acpByTask;
      return {
        sessionsByTask: { ...state.sessionsByTask, [taskId]: sessions },
        consoleByAgentId,
        errorByTask,
        acpByTask,
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
  setTaskError: (taskId, message) =>
    set((state) => {
      if (message == null) {
        if (state.errorByTask[taskId] == null) return state;
        const { [taskId]: _removed, ...errorByTask } = state.errorByTask;
        return { errorByTask };
      }
      return { errorByTask: { ...state.errorByTask, [taskId]: message } };
    }),
  setActiveTab: (taskId, localId) =>
    set((state) => ({ activeTabByTask: { ...state.activeTabByTask, [taskId]: localId } })),
  clearTaskSessions: (taskId) =>
    set((state) => {
      const { [taskId]: removed, ...sessionsByTask } = state.sessionsByTask;
      const { [taskId]: _a, ...attentionByTask } = state.attentionByTask;
      const { [taskId]: _t, ...activeTabByTask } = state.activeTabByTask;
      const { [taskId]: _s, ...setupByTask } = state.setupByTask;
      const { [taskId]: _e, ...errorByTask } = state.errorByTask;
      const { [taskId]: _acp, ...acpByTask } = state.acpByTask;
      const consoleByAgentId = { ...state.consoleByAgentId };
      for (const s of removed ?? []) if (s.backendId) delete consoleByAgentId[s.backendId];
      return { sessionsByTask, attentionByTask, activeTabByTask, consoleByAgentId, errorByTask, setupByTask, acpByTask };
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
  openUsage: () => set({ usageOpen: true }),
  closeUsage: () => set({ usageOpen: false }),
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
