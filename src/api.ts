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

// ── ACP (Agent Client Protocol) backend engine event contract ────────────────
// Mirrors `src-tauri/src/acp/events.rs::AcpEvent` exactly (tag = "type",
// camelCase variant + field names). Payloads that carry open-ended ACP schema
// data (`modes`/`models`/`content`/`rawInput`/`toolCall`) stay `unknown` on
// purpose — the Rust side deliberately keeps them as raw `serde_json::Value`
// rather than re-typing the ACP schema, so this contract doesn't silently
// shift when the ACP crate does.

export type MessageRole = "user" | "assistant";

export interface PlanEntryEvent {
  content: string;
  priority: string;
  status: string;
}

export interface CostEvent {
  amount: number;
  currency: string;
}

/** Best-effort rate-limit snapshot parsed from `UsageUpdate._meta`. Absent
 *  unless the agent reports it (only Claude does today). */
export interface RateLimitEvent {
  status: string;
  resetAt: number | null;
  limitType: string | null;
  usingOverage: boolean | null;
}

export interface PermissionOptionEvent {
  optionId: string;
  name: string;
  kind: string;
}

export type AcpEvent =
  | { type: "sessionStarted"; sessionId: string; modes: unknown | null; models: unknown | null }
  | {
      type: "messageChunk";
      role: MessageRole;
      text: string;
      messageId: string | null;
      /** Omitted (falsy) for a live chunk; `true` only for resumed-session replay. */
      replay?: boolean;
    }
  | {
      type: "thoughtChunk";
      text: string;
      messageId: string | null;
      replay?: boolean;
    }
  | {
      type: "toolCall";
      id: string;
      kind: string;
      title: string;
      status: string;
      content: unknown[];
      rawInput: unknown | null;
    }
  | {
      type: "toolCallUpdate";
      id: string;
      kind: string | null;
      title: string | null;
      status: string | null;
      content: unknown[] | null;
    }
  | { type: "plan"; entries: PlanEntryEvent[] }
  | { type: "permissionRequest"; requestId: string; options: PermissionOptionEvent[]; toolCall: unknown }
  | { type: "modeChanged"; modeId: string }
  | { type: "modelSelected"; modelId: string }
  | {
      type: "usage";
      used: number;
      size: number;
      cost: CostEvent | null;
      /** Omitted unless the agent reports rate-limit info in `_meta`. */
      rateLimit?: RateLimitEvent | null;
    }
  | { type: "turnEnded"; stopReason: string }
  | { type: "error"; message: string }
  | { type: "exit"; code: number | null };

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
  // Whether a system-sleep-preventing power assertion is currently held.
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
  // Repo the session is scoped to (orchestrator). Absent for the legacy global
  // concierge session.
  repoId?: string;
};

export function listRemoteSessions(): Promise<RemoteSession[]> {
  return invoke("list_remote_sessions");
}

/** Spawn (or reveal) the per-repo orchestrator session for `repoId`. */
export function openOrchestrator(repoId: string): Promise<void> {
  return invoke("open_orchestrator", { repoId });
}

/**
 * Open (or resume) the per-repo orchestrator session bound to a frontend
 * terminal channel, so the desktop can render + drive it. Returns the backend
 * agent id for write/resize/stop. Distinct from `openOrchestrator`,
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

/** Preview of what creating a task at the derived worktree path would do. */
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
 *  the New Task modal can warn before submit. */
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

/**
 * Persist (or clear) a repo's auto-routing policy. `policy` is the raw JSON
 * string, or null to clear. The backend rejects invalid JSON with an Err so
 * the caller can surface it.
 */
export function setRepoRoutingPolicy(repoId: string, policy: string | null): Promise<void> {
  return invoke("set_repo_routing_policy", { repoId, policy });
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

// ── ACP (Agent Client Protocol) backend engine commands ──────────────────────
// Counterparts of the PTY commands above, for tasks whose resolved agent spec
// has `execution: "acp"` (`claude-acp`/`mistral-acp`). The frontend chooses
// `startAcpAgent` vs `startAgent` by `spec.execution` — no other UI change.

/**
 * Start an ACP agent session for `taskId`, streaming structured `AcpEvent`s
 * over `onEvent`. Returns the new session's
 * agent id, used to address it in `acpPrompt`/`acpCancel`/
 * `acpRespondPermission`/`acpSetMode`/`stopSession`. Errors if the task's
 * resolved agent is a PTY engine — use `startAgent` for those instead.
 *
 * `resume` is accepted for IPC-contract symmetry with `startAgent` but is
 * **ignored in v1**: every call starts a fresh session (`session/new`).
 */
export function startAcpAgent(
  taskId: string,
  resume: boolean,
  onEvent: Channel<AcpEvent>,
  initialPrompt?: string,
): Promise<string> {
  return invoke("start_acp_agent", { taskId, resume, initialPrompt: initialPrompt ?? null, onEvent });
}

/** Send a new user prompt on a running ACP session. */
export function acpPrompt(sessionId: string, text: string): Promise<void> {
  return invoke("acp_prompt", { sessionId, text });
}

/** Cancel the in-flight turn on a running ACP session. */
export function acpCancel(sessionId: string): Promise<void> {
  return invoke("acp_cancel", { sessionId });
}

/**
 * Answer a pending `permissionRequest` event: `optionId` selects that option,
 * omitted cancels the request.
 */
export function acpRespondPermission(
  sessionId: string,
  requestId: string,
  optionId?: string,
): Promise<void> {
  return invoke("acp_respond_permission", { sessionId, requestId, optionId: optionId ?? null });
}

/** Switch a running ACP session's active mode. */
export function acpSetMode(sessionId: string, modeId: string): Promise<void> {
  return invoke("acp_set_mode", { sessionId, modeId });
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

export interface AgentError {
  agentId: string;
  taskId: string;
  /** The error reason; absent/null means "clear the stored error for this task". */
  message?: string | null;
}

/** Subscribe to per-task agent error updates (StopFailure reason, or clear). */
export function onAgentError(cb: (e: AgentError) => void): Promise<UnlistenFn> {
  return listen<AgentError>("agent_error", (event) => cb(event.payload));
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
    // The scheduler sets this to skip prepending the repo prompt at fire time.
    skipRepoPrompt?: boolean;
    // Remote can ask the desktop-owned terminal path to resume an existing task.
    resume?: boolean;
  }) => void,
): Promise<UnlistenFn> {
  return listen<{ taskId: string; initialPrompt?: string | null; skipRepoPrompt?: boolean; resume?: boolean }>(
    "task_launched",
    (event) => cb(event.payload),
  );
}

// The user picked a task from the system-tray menu. Rust has already brought
// the window to the front; the payload names which task to select.
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

// ── Schedule API wrappers ────────────────────────────────────────────────────

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
  /// Skip prepending the repo's initial prompt when this schedule fires.
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

// ── ACP per-model usage / cost / rate-limit summary ─────────────────────────

/** Per-(provider, model) spend rollup over the window. `cost` is the summed
 *  cumulative session cost; `contextPeak`/`contextSize` are a context-window
 *  gauge (not cumulative tokens); `rateStatus` is the latest best-effort quota
 *  status seen. */
export interface ModelUsage {
  provider: string;
  model: string | null;
  cost: number;
  currency: string | null;
  contextPeak: number;
  contextSize: number;
  sessions: number;
  rateStatus: string | null;
}

export interface TaskUsage {
  taskId: string;
  cost: number;
  currency: string | null;
}

export interface AcpUsageSummary {
  totalCost: number;
  currency: string | null;
  sessions: number;
  byModel: ModelUsage[];
  byTask: TaskUsage[];
}

/** Per-model usage/cost/rate-limit summary over the last `windowDays` (default
 *  7 on the backend). */
export function getAcpUsageSummary(windowDays?: number): Promise<AcpUsageSummary> {
  return invoke("get_acp_usage_summary", { windowDays });
}
