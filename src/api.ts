import { invoke, Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
  onAction,
} from "@tauri-apps/plugin-notification";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { createNotificationRegistry } from "./notify/registry";
import { openUrl as _openUrl } from "@tauri-apps/plugin-opener";
import type { Repo, Task, AgentSpec, AgentActivity, SetupStatus } from "./store";
import type { CustomSound } from "./sound/types";

export type ChangeKind =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "copied"
  | "type_changed"
  | "unknown";

export interface FileChange {
  path: string;
  change: ChangeKind;
}

export type PtyEvent =
  | { type: "data"; data: string }
  | { type: "exit"; code: number };

export interface AppSnapshot {
  repos: Repo[];
  tasks: Task[];
  worktreesRoot: string;
}

export function listState(): Promise<AppSnapshot> {
  return invoke("list_state");
}

export type RemoteStatus = {
  active: boolean;
  token?: string | null;
  url?: string | null;
  // Whether a system-sleep-preventing power assertion is currently held (TASK-104).
  sleepInhibited: boolean;
};

export function enableRemote(): Promise<RemoteStatus> {
  return invoke("enable_remote");
}
export function disableRemote(): Promise<RemoteStatus> {
  return invoke("disable_remote");
}
export function remoteStatus(): Promise<RemoteStatus> {
  return invoke("remote_status");
}

export type RemoteSession = {
  id: string;
  kind: string;
  idleSecs: number;
  // Repo the session is scoped to (TASK-180 orchestrator). Absent for the legacy
  // global concierge session.
  repoId?: string;
};

export function listRemoteSessions(): Promise<RemoteSession[]> {
  return invoke("list_remote_sessions");
}

/** Spawn (or reveal) the per-repo orchestrator session for `repoId` (TASK-180). */
export function openOrchestrator(repoId: string): Promise<void> {
  return invoke("open_orchestrator", { repoId });
}

/**
 * Open (or resume) the per-repo orchestrator session bound to a frontend
 * terminal channel, so the desktop can render + drive it (TASK-126). Returns the
 * backend agent id for write/resize/stop. Distinct from `openOrchestrator`,
 * which spawns the session sink-drained for the mobile/remote path.
 */
export function openOrchestratorTerminal(
  repoId: string,
  onEvent: Channel<PtyEvent>,
): Promise<string> {
  return invoke("open_orchestrator_terminal", { repoId, onEvent });
}

export function addRepo(path: string): Promise<Repo> {
  return invoke("add_repo", { path });
}

export function updateRepo(
  repoId: string,
  name: string,
  defaultBranch: string,
  worktreeRoot: string | null,
  setupCommand: string | null,
  autoStartAgent: boolean,
  initialPrompt: string | null,
  soundSettings: string | null,
  fetchRemoteBase: boolean | null = null,
  defaultAgent: string | null = null,
  autoApprove: boolean | null = null,
  inPlaceDefault = false,
): Promise<Repo> {
  return invoke("update_repo", {
    repoId,
    name,
    defaultBranch,
    worktreeRoot: worktreeRoot ?? null,
    setupCommand: setupCommand ?? null,
    autoStartAgent,
    initialPrompt: initialPrompt ?? null,
    soundSettings,
    fetchRemoteBase,
    defaultAgent,
    autoApprove,
    inPlaceDefault,
  });
}

export function setSoundSettings(settings: string): Promise<void> {
  return invoke("set_sound_settings", { settings });
}

export function setFetchRemoteBase(enabled: boolean): Promise<void> {
  return invoke("set_fetch_remote_base", { enabled });
}

export function setInjectLavigieSkills(enabled: boolean): Promise<void> {
  return invoke("set_inject_lavigie_skills", { enabled });
}

/** True when the user is in a meeting (mic/camera active). macOS-only; false elsewhere. */
export function isMeetingActive(): Promise<boolean> {
  return invoke("is_meeting_active");
}

export function importCustomSound(srcPath: string, label: string): Promise<CustomSound> {
  return invoke("import_custom_sound", { srcPath, label });
}

export function listCustomSounds(): Promise<CustomSound[]> {
  return invoke("list_custom_sounds");
}

export function readSoundBytes(id: string): Promise<number[]> {
  return invoke("read_sound_bytes", { id });
}

export function deleteCustomSound(id: string): Promise<void> {
  return invoke("delete_custom_sound", { id });
}

export function removeRepo(repoId: string): Promise<void> {
  return invoke("remove_repo", { repoId });
}

export function listRepoBranches(repoId: string): Promise<string[]> {
  return invoke("list_repo_branches", { repoId });
}

export function createTask(
  repoId: string,
  title: string,
  baseBranch?: string,
  ticketKey?: string,
  agent?: string,
  model?: string | null,
  autoApprove: boolean | null = null,
  inPlace = false,
  branchName: string | null = null,
): Promise<Task> {
  return invoke("create_task", {
    args: {
      repoId,
      title,
      baseBranch: baseBranch ?? null,
      ticketKey: ticketKey ?? null,
      agent: agent ?? null,
      model: model ?? null,
      autoApprove: autoApprove ?? null,
      inPlace,
      branchName: branchName ?? null,
    },
  });
}

/** Preview of what creating a task at the derived worktree path would do (TASK-125). */
export interface WorktreePreview {
  /**
   * - "vacant"       — path free, create normally (no message).
   * - "reuse-branch" — path free but the branch exists; its commits are reused.
   * - "adopt"        — an existing worktree on the branch will be reused.
   * - "reclaim"      — a leftover/orphaned worktree will be cleaned up & recreated.
   * - "conflict"     — the path is occupied by a mismatch; creation would fail.
   */
  state: "vacant" | "reuse-branch" | "adopt" | "reclaim" | "conflict";
  path: string;
  message: string | null;
}

/** Check whether the worktree path derived from these inputs already exists, so
 *  the New Task modal can warn before submit (TASK-125). */
export function checkWorktreePath(
  repoId: string,
  title: string,
  baseBranch?: string,
  ticketKey?: string,
): Promise<WorktreePreview> {
  return invoke("check_worktree_path", {
    repoId,
    title,
    baseBranch: baseBranch ?? null,
    ticketKey: ticketKey ?? null,
  });
}

export function listAgents(): Promise<AgentSpec[]> {
  return invoke("list_agents");
}

export function upsertCustomAgent(spec: AgentSpec): Promise<void> {
  return invoke("upsert_custom_agent", { spec });
}

export function deleteCustomAgent(name: string): Promise<void> {
  return invoke("delete_custom_agent", { name });
}

export function setTaskAgent(taskId: string, agent: string | null): Promise<void> {
  return invoke("set_task_agent", { taskId, agent });
}

export function setRepoDefaultModel(repoId: string, model: string | null): Promise<void> {
  return invoke("set_repo_default_model", { repoId, model });
}

export function listAgentModels(agentName: string): Promise<string[]> {
  return invoke("list_agent_models", { agentName });
}

export function setTaskModel(taskId: string, model: string | null): Promise<void> {
  return invoke("set_task_model", { taskId, model });
}

export function setTaskAutoApprove(
  taskId: string,
  autoApprove: boolean | null,
): Promise<void> {
  return invoke("set_task_auto_approve", { taskId, autoApprove });
}

export function deleteTask(taskId: string, deleteBranch: boolean): Promise<void> {
  return invoke("delete_task", { taskId, deleteBranch });
}

export function setTaskHidden(taskId: string, hidden: boolean): Promise<void> {
  return invoke("set_task_hidden", { taskId, hidden });
}

export function startAgent(
  taskId: string,
  resume: boolean,
  onEvent: Channel<PtyEvent>,
  initialPrompt?: string,
): Promise<string> {
  return invoke("start_agent", { taskId, resume, initialPrompt: initialPrompt ?? null, onEvent });
}

export function startShell(taskId: string, onEvent: Channel<PtyEvent>): Promise<string> {
  return invoke("start_shell", { taskId, onEvent });
}

export function writeSession(sessionId: string, data: string): Promise<void> {
  return invoke("write_session", { sessionId, data });
}

export function resizeSession(
  sessionId: string,
  cols: number,
  rows: number,
): Promise<void> {
  return invoke("resize_session", { sessionId, cols, rows });
}

export function stopSession(sessionId: string): Promise<void> {
  return invoke("stop_session", { sessionId });
}

export function onAgentStatus(
  cb: (e: { agentId: string; status: AgentActivity }) => void,
): Promise<UnlistenFn> {
  return listen<{ agentId: string; status: AgentActivity }>(
    "agent_status",
    (event) => cb(event.payload),
  );
}

export interface AgentConsole {
  agentId: string;
  model?: string;
  contextRemainingPercent?: number;
  mode?: string;
}

export function onAgentConsole(cb: (e: AgentConsole) => void): Promise<UnlistenFn> {
  return listen<AgentConsole>("agent_console", (event) => cb(event.payload));
}

export function onTaskRenamed(
  cb: (e: { taskId: string; title: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string; title: string }>("task_renamed", (event) => cb(event.payload));
}

export function onTaskRemoved(
  cb: (e: { taskId: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string }>("task_removed", (event) => cb(event.payload));
}

export function onTaskCreated(
  cb: (e: { taskId: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string }>("task_created", (event) => cb(event.payload));
}

export function getSetupState(
  taskId: string,
): Promise<{ status: SetupStatus | null; log: string; exitCode: number | null }> {
  return invoke("get_setup_state", { taskId });
}

export function onSetupOutput(
  cb: (e: { taskId: string; data: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string; data: string }>("setup_output", (event) => cb(event.payload));
}

export function onSetupStatus(
  cb: (e: { taskId: string; status: SetupStatus; exitCode: number | null }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string; status: SetupStatus; exitCode: number | null }>(
    "setup_status",
    (event) => cb(event.payload),
  );
}

export function onTaskLaunched(
  cb: (e: {
    taskId: string;
    initialPrompt?: string | null;
    // TASK-181: scheduler sets this to skip prepending the repo prompt at fire time.
    skipRepoPrompt?: boolean;
  }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string; initialPrompt?: string | null; skipRepoPrompt?: boolean }>(
    "task_launched",
    (event) => cb(event.payload),
  );
}

// TASK-204: the user picked a task from the system-tray menu. Rust has already
// brought the window to the front; the payload names which task to select.
export function onTraySelectTask(
  cb: (e: { taskId: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string }>("tray_select_task", (event) => cb(event.payload));
}

export type WebviewDropPayload =
  | { type: "enter"; paths: string[]; position: { x: number; y: number } }
  | { type: "over"; position: { x: number; y: number } }
  | { type: "drop"; paths: string[]; position: { x: number; y: number } }
  | { type: "leave" };

// Subscribe to native OS file drops on the webview. Tauri intercepts the drop and
// gives us the file paths; position is in physical pixels.
export function onWebviewFileDrop(
  handler: (payload: WebviewDropPayload) => void,
): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent((event) =>
    handler(event.payload as WebviewDropPayload),
  );
}

const notificationRegistry = createNotificationRegistry();
let notificationFocusHandler: ((taskId: string) => void) | undefined;
let actionListenerRegistered = false;

/** Register the callback invoked when the user taps a notification (routes to its task). */
export function setNotificationFocusHandler(fn: (taskId: string) => void): void {
  notificationFocusHandler = fn;
}

/** Lazily subscribe to notification taps. Degrades to plain popups if unsupported. */
async function ensureActionListener(): Promise<void> {
  if (actionListenerRegistered) return;
  actionListenerRegistered = true;
  try {
    await onAction((notification) => {
      const taskId =
        notification.id != null ? notificationRegistry.resolve(notification.id) : undefined;
      if (taskId) notificationFocusHandler?.(taskId);
      const win = getCurrentWindow();
      void win.unminimize().catch(() => {});
      void win.setFocus().catch(() => {});
    });
  } catch {
    // onAction may be unavailable on some targets — leave it off and keep firing popups.
    actionListenerRegistered = false;
  }
}

export interface AgentEventNotification {
  title: string;
  body: string;
  taskId: string;
}

/** Fire a rich OS notification for an agent lifecycle event, tagged so a tap
 *  can route back to the owning task. */
export async function notifyAgentEvent({
  title,
  body,
  taskId,
}: AgentEventNotification): Promise<void> {
  let granted = await isPermissionGranted();
  if (!granted) {
    const permission = await requestPermission();
    granted = permission === "granted";
  }
  if (!granted) return;
  void ensureActionListener();
  const id = notificationRegistry.register(taskId);
  await sendNotification({ id, title, body });
}

// Review scope: "uncommitted" = working tree vs HEAD (commit-able);
// "base" = the whole branch diff vs the base branch (read-only).
export type DiffScope = "uncommitted" | "base";

export function getDiff(
  taskId: string,
  scope: DiffScope = "uncommitted",
): Promise<string> {
  return invoke("get_diff", { taskId, scope });
}

export function getChangedFiles(
  taskId: string,
  scope: DiffScope = "uncommitted",
): Promise<FileChange[]> {
  return invoke("get_changed_files", { taskId, scope });
}

export function stageFiles(taskId: string, paths: string[]): Promise<void> {
  return invoke("stage_files", { taskId, paths });
}

export function commitTask(taskId: string, message: string): Promise<void> {
  return invoke("commit_task", { taskId, message });
}

export function finishTask(taskId: string, mode: "keep" | "discard" | "merge"): Promise<void> {
  return invoke("finish_task", { taskId, mode });
}

// ── PR types ──────────────────────────────────────────────────────────────────

export type PrCheckStatus = "success" | "failure" | "pending" | "neutral";

export interface PrCheck {
  name: string;
  status: PrCheckStatus;
}

export interface PrStatus {
  number: number;
  url: string;
  title: string;
  state: string;
  isDraft: boolean;
  mergeable: string;
  reviewDecision: string | null;
  checks: PrCheck[];
}

export interface PrComment {
  author: string;
  body: string;
  createdAt: string;
  path: string | null;
  line: number | null;
  kind: "issue_comment" | "review" | "inline";
  state: string | null;
}

export interface GhStatus {
  available: boolean;
  authenticated: boolean;
}

export interface CreatePrResult {
  number: number;
  url: string;
}

// ── PR API wrappers ───────────────────────────────────────────────────────────

export function ghStatus(): Promise<GhStatus> {
  return invoke("gh_status");
}

export function createPr(
  taskId: string,
  title: string,
  body: string,
  draft: boolean,
): Promise<CreatePrResult> {
  return invoke("create_pr", { taskId, title, body, draft });
}

export function getPrStatus(taskId: string): Promise<PrStatus | null> {
  return invoke("get_pr_status", { taskId });
}

export function getPrComments(taskId: string): Promise<PrComment[]> {
  return invoke("get_pr_comments", { taskId });
}

export function openUrl(url: string): Promise<void> {
  return _openUrl(url);
}

// ── Prompt library API wrappers ──────────────────────────────────────────────

export interface Prompt {
  id: string;
  label: string;
  body: string;
  position: number;
}

export function listPrompts(): Promise<Prompt[]> {
  return invoke("list_prompts");
}

export function createPrompt(label: string, body: string): Promise<Prompt> {
  return invoke("create_prompt", { label, body });
}

export function updatePrompt(id: string, label: string, body: string): Promise<void> {
  return invoke("update_prompt", { id, label, body });
}

export function deletePrompt(id: string): Promise<void> {
  return invoke("delete_prompt", { id });
}

export function reorderPrompts(orderedIds: string[]): Promise<void> {
  return invoke("reorder_prompts", { orderedIds });
}

// ── Task docs API wrappers ───────────────────────────────────────────────────

export interface DocRef {
  id: string;
  label: string;
}

export function listTaskDocs(taskId: string): Promise<DocRef[]> {
  return invoke("list_task_docs", { taskId });
}

export function readTaskDoc(taskId: string, id: string): Promise<string> {
  return invoke("read_task_doc", { taskId, id });
}

// ── Schedule API wrappers (TASK-173) ──────────────────────────────────────────

export interface Schedule {
  id: string;
  repoId: string;
  name: string;
  prompt: string;
  cron: string;
  agent: string | null;
  model: string | null;
  baseBranch: string | null;
  enabled: boolean;
  oneShot: boolean;
  /// TASK-181: skip prepending the repo's initial prompt when this schedule fires.
  skipRepoPrompt: boolean;
  nextRunAt: number | null;
  lastRunAt: number | null;
  createdAt: number;
  updatedAt: number;
}

export function listSchedules(repoId: string): Promise<Schedule[]> {
  return invoke("list_schedules", { repoId });
}

export function createSchedule(input: {
  repoId: string;
  name: string;
  prompt: string;
  cron: string;
  agent?: string | null;
  model?: string | null;
  baseBranch?: string | null;
  skipRepoPrompt?: boolean;
}): Promise<Schedule> {
  return invoke("create_schedule", {
    repoId: input.repoId,
    name: input.name,
    prompt: input.prompt,
    cron: input.cron,
    agent: input.agent ?? null,
    model: input.model ?? null,
    baseBranch: input.baseBranch ?? null,
    skipRepoPrompt: input.skipRepoPrompt ?? null,
  });
}

export function createOneShotSchedule(input: {
  repoId: string;
  name: string;
  prompt: string;
  inSeconds?: number | null;
  atUnix?: number | null;
  agent?: string | null;
  model?: string | null;
  baseBranch?: string | null;
  skipRepoPrompt?: boolean;
}): Promise<Schedule> {
  return invoke("create_one_shot_schedule", {
    repoId: input.repoId,
    name: input.name,
    prompt: input.prompt,
    inSeconds: input.inSeconds ?? null,
    atUnix: input.atUnix ?? null,
    agent: input.agent ?? null,
    model: input.model ?? null,
    baseBranch: input.baseBranch ?? null,
    skipRepoPrompt: input.skipRepoPrompt ?? null,
  });
}

export function updateSchedule(input: {
  id: string;
  name: string;
  prompt: string;
  cron: string;
  agent: string | null;
  model: string | null;
  baseBranch: string | null;
  enabled: boolean;
  skipRepoPrompt: boolean;
}): Promise<Schedule> {
  return invoke("update_schedule", input);
}

export function setScheduleEnabled(id: string, enabled: boolean): Promise<Schedule> {
  return invoke("set_schedule_enabled", { id, enabled });
}

export function deleteSchedule(id: string): Promise<void> {
  return invoke("delete_schedule", { id });
}

export function previewNextRun(cron: string): Promise<number> {
  return invoke("preview_next_run", { cron });
}
