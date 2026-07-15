import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { addRepo, createTask, checkWorktreePath, createOneShotSchedule, type WorktreePreview } from "../../api";
import { useVigieStore } from "../../store";
import type { Repo } from "../../store";
import { taskName } from "../../lib/taskName";
import { combineInitialPrompts } from "../../lib/combineInitialPrompts";
import { useAgents } from "../../hooks/useAgents";
import { AgentModelPicker } from "../Agent/AgentModelPicker";
import { PromptPicker } from "../Prompts/PromptPicker";
import { insertAtCursor } from "../Prompts/insertAtCursor";
import { StatusDot } from "../StatusDot/StatusDot";
import { RepoSettingsModal } from "./RepoSettingsModal";
import { ContextMenu, type ContextMenuItem } from "../ContextMenu/ContextMenu";
import { DeleteTaskModal } from "./DeleteTaskModal";
import "./Sidebar.css";

interface NewTaskFormProps {
  repo: Repo;
  onClose: () => void;
}

export function NewTaskForm({ repo, onClose }: NewTaskFormProps) {
  const refresh = useVigieStore((state) => state.refresh);
  const setSelectedTask = useVigieStore((state) => state.setSelectedTask);
  const startAgentSession = useVigieStore((state) => state.startAgentSession);
  const openSettings = useVigieStore((s) => s.openSettings);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const [title, setTitle] = useState("");
  const [ticketId, setTicketId] = useState("");
  const [baseBranch, setBaseBranch] = useState("");
  const [taskPrompt, setTaskPrompt] = useState("");
  // TASK-160: launch-time-only opt-out of the repo-level prompt for this task.
  // Only meaningful when the repo actually has a non-empty initialPrompt.
  const [skipRepoPrompt, setSkipRepoPrompt] = useState(false);
  const hasRepoPrompt = (repo.initialPrompt ?? "").trim().length > 0;
  // Per-task launch toggle; defaults from the repo's auto-start setting and can
  // be overridden per task. This (not the repo setting directly) gates launch.
  const [startImmediately, setStartImmediately] = useState(repo.autoStartAgent ?? false);
  // TASK-179: launch-time deferred one-shot — mutually exclusive with "start immediately".
  const [startLater, setStartLater] = useState(false);
  const [startLaterHours, setStartLaterHours] = useState("3");
  const [agentName, setAgentName] = useState(repo.defaultAgent ?? "claude");
  const [modelName, setModelName] = useState<string | null>(repo.defaultModel ?? null);
  const [autoApprove, setAutoApprove] = useState<boolean | null>(null);
  const { agents } = useAgents();
  const [phase, setPhase] = useState<"form" | "running">("form");
  const [error, setError] = useState<string | null>(null);
  // TASK-125: warn when the derived worktree path already exists on disk.
  const [worktreePreview, setWorktreePreview] = useState<WorktreePreview | null>(null);
  // TASK-163: run this task in the repo's existing checkout (no worktree).
  const [inPlace, setInPlace] = useState(repo.inPlaceDefault ?? false);
  const [inPlaceBranch, setInPlaceBranch] = useState("");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Debounced worktree-path check (TASK-125): as the title/ticket/base change,
  // ask the backend whether the derived worktree path already exists. Only a
  // non-vacant preview (adopt/conflict) is surfaced; a stale in-flight request
  // is discarded via the `cancelled` guard.
  useEffect(() => {
    if (inPlace) {
      setWorktreePreview(null);
      return;
    }
    const t = title.trim();
    const tk = ticketId.trim();
    if (!t && !tk) {
      setWorktreePreview(null);
      return;
    }
    let cancelled = false;
    const handle = setTimeout(() => {
      checkWorktreePath(repo.id, t, baseBranch.trim() || undefined, tk || undefined)
        .then((preview) => {
          if (!cancelled) setWorktreePreview(preview.state === "vacant" ? null : preview);
        })
        .catch(() => {
          if (!cancelled) setWorktreePreview(null);
        });
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
  }, [title, ticketId, baseBranch, repo.id, inPlace]);

  const selectedAgent = agents.find((a) => a.name === agentName);

  const insertPrompt = (body: string) => {
    const el = promptRef.current;
    const start = el?.selectionStart ?? taskPrompt.length;
    const end = el?.selectionEnd ?? taskPrompt.length;
    const { value, cursor } = insertAtCursor(taskPrompt, start, end, body);
    setTaskPrompt(value);
    // restore focus + caret after React re-renders the controlled value
    requestAnimationFrame(() => {
      if (promptRef.current) {
        promptRef.current.focus();
        promptRef.current.setSelectionRange(cursor, cursor);
      }
    });
  };

  const runCreate = async () => {
    setPhase("running");
    setError(null);
    try {
      if (startLater) {
        const hours = Number(startLaterHours);
        if (!Number.isFinite(hours) || hours <= 0) {
          throw new Error("Enter a positive number of hours.");
        }
        // Deferred: create a one-shot schedule instead of a task now. We store
        // the RAW task prompt (matching MCP start_task / recurring); the fire-time
        // launch combines repo.initialPrompt via useTaskLaunch unless skipped.
        // TASK-181: thread the TASK-160 skip checkbox so the deferred run honors the
        // same include/skip choice as the immediate path (Sidebar create below).
        await createOneShotSchedule({
          repoId: repo.id,
          name: title.trim() || ticketId.trim(),
          prompt: taskPrompt,
          inSeconds: Math.round(hours * 3600),
          agent: agentName,
          model: modelName,
          baseBranch: baseBranch.trim() || null,
          skipRepoPrompt: hasRepoPrompt ? skipRepoPrompt : false,
        });
        onClose();
        return;
      }
      const task = await createTask(
        repo.id,
        title.trim(),
        baseBranch.trim() || undefined,
        ticketId.trim() || undefined,
        agentName,
        modelName,
        autoApprove,
        inPlace,
        inPlace ? inPlaceBranch.trim() || null : null,
      );
      await refresh();
      setSelectedTask(task.id);
      if (startImmediately) {
        startAgentSession(
          task.id,
          false,
          selectedAgent
            ? { label: selectedAgent.displayName, lifecycle: selectedAgent.status === "lifecycle" }
            : undefined,
          combineInitialPrompts(skipRepoPrompt ? null : repo.initialPrompt, taskPrompt),
        );
      }
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!title.trim() && !ticketId.trim()) return;
    runCreate();
  };

  if (phase === "running") {
    return (
      <div className="new-task-modal__backdrop" role="presentation">
        <div className="new-task-modal" role="dialog" aria-label={`New task in ${repo.name}`}>
          <header className="new-task-modal__header">
            <h2 className="new-task-modal__title">New task</h2>
          </header>
          <div className="new-task-modal__body new-task-form--running">
            <p className="new-task-form__status">Creating {title.trim() || ticketId.trim()}…</p>
            {error && <p className="new-task-form__error" role="alert">{error}</p>}
          </div>
          {error && (
            <footer className="new-task-modal__footer">
              <button type="button" className="btn btn--ghost" onClick={onClose}>Cancel</button>
              <button type="button" className="btn btn--primary" onClick={runCreate}>Retry</button>
            </footer>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="new-task-modal__backdrop" role="presentation" onClick={onClose}>
      <div
        className="new-task-modal"
        role="dialog"
        aria-label={`New task in ${repo.name}`}
        onClick={(e) => e.stopPropagation()}
      >
        <form onSubmit={handleSubmit}>
          <header className="new-task-modal__header">
            <h2 className="new-task-modal__title">New task</h2>
            <button
              type="button"
              className="new-task-modal__close"
              onClick={onClose}
              aria-label="Close"
            >
              ✕
            </button>
          </header>

          <div className="new-task-modal__body">
            <div className="new-task-form__group">
              <span className="new-task-form__label">Repository</span>
              <div className="new-task-form__readonly">{repo.name}</div>
            </div>

            <div className="new-task-form__row">
              <label className="new-task-form__group">
                <span className="new-task-form__label">Task name</span>
                <input
                  type="text"
                  className="field"
                  placeholder="e.g. Add People CRUD tools"
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  aria-label="Task title"
                  autoFocus
                />
              </label>
              <label className="new-task-form__group">
                <span className="new-task-form__label">Ticket ID</span>
                <input
                  type="text"
                  className="field"
                  placeholder="optional (e.g. TST-1)"
                  value={ticketId}
                  onChange={(e) => setTicketId(e.target.value)}
                  aria-label="Ticket ID (optional)"
                />
              </label>
            </div>

            <label className="new-task-form__group">
              <span className="new-task-form__label">Base branch</span>
              <input
                type="text"
                className="field"
                placeholder={`optional (default: ${repo.defaultBranch})`}
                value={baseBranch}
                onChange={(e) => setBaseBranch(e.target.value)}
                aria-label="Base branch (optional)"
              />
            </label>

            {worktreePreview && (
              <p
                className={`new-task-form__worktree-warning new-task-form__worktree-warning--${worktreePreview.state}`}
                role="status"
              >
                {worktreePreview.message}
              </p>
            )}

            {/* Not a <label>: it contains the PromptPicker button, and a label
                forwards clicks to its first labelable descendant (that button),
                re-opening the dropdown after every selection. The textarea keeps
                its aria-label so it stays accessible. */}
            <div className="new-task-form__group">
              <span className="new-task-form__label new-task-form__label--with-action">
                <span>
                  Prompt <span className="new-task-form__label-hint">— sent to the agent on start</span>
                </span>
                <PromptPicker onSelect={insertPrompt} onManage={openSettings} />
              </span>
              <textarea
                ref={promptRef}
                className="field new-task-form__prompt"
                placeholder="Describe what the agent should do…"
                value={taskPrompt}
                onChange={(e) => setTaskPrompt(e.target.value)}
                aria-label="Initial prompt"
                rows={4}
              />
            </div>

            {hasRepoPrompt && (
              <label className="new-task-form__checkbox">
                <input
                  type="checkbox"
                  checked={skipRepoPrompt}
                  onChange={(e) => setSkipRepoPrompt(e.target.checked)}
                  aria-label="Skip the repository prompt for this task"
                />
                <span>
                  Skip the repository prompt for this task
                  {/* Native `title` tooltips don't render in the macOS webview,
                      so the hint is a CSS ::after driven by data-tooltip, shown
                      on hover and keyboard focus. */}
                  <span
                    className="new-task-form__info"
                    tabIndex={0}
                    role="img"
                    aria-label="Otherwise the repository prompt is prepended to the prompt above."
                    data-tooltip="Otherwise the repository prompt is prepended to the prompt above."
                    // The icon lives inside the <label>; without this a click on
                    // it would activate the label and toggle the checkbox.
                    onClick={(e) => e.preventDefault()}
                  >
                    ⓘ
                  </span>
                </span>
              </label>
            )}

            <div className="new-task-form__group">
              <AgentModelPicker
                agent={agentName}
                model={modelName}
                onChange={(a, m) => { setAgentName(a); setModelName(m); }}
              />
            </div>

            <label className="new-task-form__group">
              <span className="new-task-form__label">Auto-approve</span>
              <select
                className="field"
                aria-label="Auto-approve for new task"
                value={autoApprove == null ? "inherit" : autoApprove ? "on" : "off"}
                onChange={(e) =>
                  setAutoApprove(
                    e.target.value === "inherit" ? null : e.target.value === "on",
                  )
                }
              >
                <option value="inherit">Inherit from repo</option>
                <option value="on">On</option>
                <option value="off">Off</option>
              </select>
            </label>

            <label className="new-task-form__checkbox">
              <input
                type="checkbox"
                checked={inPlace}
                disabled={startLater}
                onChange={(e) => setInPlace(e.target.checked)}
                aria-label="Work in place (no worktree)"
              />
              <span>
                Work in place — no worktree
                <span
                  className="new-task-form__info"
                  tabIndex={0}
                  role="img"
                  aria-label="Runs the agent in the repo's existing checkout instead of a git worktree. Only one in-place task per repo."
                  data-tooltip="Runs the agent in the repo's existing checkout instead of a git worktree. Only one in-place task per repo."
                  onClick={(e) => e.preventDefault()}
                >
                  ⓘ
                </span>
              </span>
            </label>

            {inPlace && !startLater && (
              <label className="new-task-form__group">
                <span className="new-task-form__label">New branch name</span>
                <input
                  type="text"
                  className="field"
                  placeholder="optional — blank uses the current branch"
                  value={inPlaceBranch}
                  onChange={(e) => setInPlaceBranch(e.target.value)}
                  aria-label="New branch name"
                />
              </label>
            )}

            <label className="new-task-form__checkbox">
              <input
                type="checkbox"
                checked={startImmediately}
                disabled={startLater}
                onChange={(e) => setStartImmediately(e.target.checked)}
                aria-label="Start the agent immediately"
              />
              <span>Start the agent immediately</span>
            </label>

            <label className="new-task-form__checkbox">
              <input
                type="checkbox"
                checked={startLater}
                onChange={(e) => {
                  setStartLater(e.target.checked);
                  if (e.target.checked) {
                    setStartImmediately(false);
                    setInPlace(false);
                  }
                }}
                aria-label="Start the agent later"
              />
              <span>Start later (deferred one-shot)</span>
            </label>

            {startLater && (
              <label className="new-task-form__group">
                <span className="new-task-form__label">Start in hours</span>
                <input
                  type="number"
                  min="0"
                  step="0.5"
                  className="field"
                  aria-label="Start in hours"
                  value={startLaterHours}
                  onChange={(e) => setStartLaterHours(e.target.value)}
                />
                <span className="new-task-form__label-hint">
                  {(() => {
                    const h = Number(startLaterHours);
                    if (!Number.isFinite(h) || h <= 0) return "Enter a positive number of hours.";
                    return `Launches once at ${new Date(Date.now() + h * 3600 * 1000).toLocaleString()}. The repository prompt still applies.`;
                  })()}
                </span>
              </label>
            )}
          </div>

          <footer className="new-task-modal__footer">
            <button type="button" className="btn btn--ghost" onClick={onClose}>Cancel</button>
            <button type="submit" className="btn btn--primary">Create task</button>
          </footer>
        </form>
      </div>
    </div>
  );
}

interface RepoSectionProps {
  repo: Repo;
  search: string;
}

export function RepoSection({ repo, search }: RepoSectionProps) {
  const allTasks = useVigieStore((state) => state.tasks);
  const selectedTaskId = useVigieStore((state) => state.selectedTaskId);
  const setSelectedTask = useVigieStore((state) => state.setSelectedTask);
  const selectedOrchestratorRepoId = useVigieStore((state) => state.selectedOrchestratorRepoId);
  const setSelectedOrchestrator = useVigieStore((state) => state.setSelectedOrchestrator);
  const startOrchestratorSession = useVigieStore((state) => state.startOrchestratorSession);
  const sessionsByTask = useVigieStore((state) => state.sessionsByTask);
  const attentionByTask = useVigieStore((state) => state.attentionByTask);
  const setupByTask = useVigieStore((state) => state.setupByTask);
  const deleteTask = useVigieStore((state) => state.deleteTask);
  const hideTask = useVigieStore((state) => state.hideTask);
  const reopenTask = useVigieStore((state) => state.reopenTask);
  const [collapsed, setCollapsed] = useState(false);
  const [hiddenCollapsed, setHiddenCollapsed] = useState(true);
  const [showNewTaskForm, setShowNewTaskForm] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [menu, setMenu] = useState<{ taskId: string; x: number; y: number } | null>(null);
  const [hiddenMenu, setHiddenMenu] = useState<{ taskId: string; x: number; y: number } | null>(null);
  const [confirmTaskId, setConfirmTaskId] = useState<string | null>(null);

  const query = search.trim().toLowerCase();
  
  const allRepoTasks = allTasks.filter((task) => task.repoId === repo.id);
  const activeTasks = allRepoTasks.filter((task) => !task.hidden);
  const hiddenTasks = allRepoTasks.filter((task) => task.hidden);
  
  const tasks = activeTasks.filter(
    (task) =>
      !query ||
      task.title.toLowerCase().includes(query) ||
      (task.ticketKey?.toLowerCase().includes(query) ?? false),
  );
  const filteredHiddenTasks = hiddenTasks.filter(
    (task) =>
      !query ||
      task.title.toLowerCase().includes(query) ||
      (task.ticketKey?.toLowerCase().includes(query) ?? false),
  );

  // While searching, hide repos that have no matching task to reduce noise.
  if (query && tasks.length === 0 && filteredHiddenTasks.length === 0) return null;

  return (
    <li className="sidebar__repo">
      <div className="sidebar__repo-header">
        <button
          type="button"
          className="sidebar__repo-toggle"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((v) => !v)}
        >
          <span
            className={
              "sidebar__caret" + (collapsed ? " sidebar__caret--collapsed" : "")
            }
            aria-hidden
          >
            ▾
          </span>
          <span className="sidebar__repo-name">{repo.name}</span>
        </button>
        <button
          type="button"
          className="sidebar__new-task-button"
          onClick={() => setShowNewTaskForm((v) => !v)}
        >
          New task
        </button>
        <button
          type="button"
          className="sidebar__repo-settings-button"
          aria-label={`Settings for ${repo.name}`}
          onClick={() => setShowSettings(true)}
        >
          ⚙
        </button>
      </div>
      <button
        type="button"
        className={
          "sidebar__orchestrator-row" +
          (selectedOrchestratorRepoId === repo.id ? " sidebar__orchestrator-row--selected" : "")
        }
        onClick={() => {
          setSelectedOrchestrator(repo.id);
          startOrchestratorSession(repo.id);
        }}
      >
        <span className="sidebar__orchestrator-icon" aria-hidden>◇</span>
        <span className="sidebar__orchestrator-label">Orchestrator</span>
      </button>
      {showNewTaskForm && (
        <NewTaskForm repo={repo} onClose={() => setShowNewTaskForm(false)} />
      )}
      {!collapsed && (
        <ul className="sidebar__tasks">
          {tasks.map((task) => {
            const agentSession = sessionsByTask[task.id]?.find((s) => s.kind === "agent");
            const dotStatus = agentSession?.activity ?? task.status;
            const selected = task.id === selectedTaskId;
            const needsAttention = !selected && !!attentionByTask[task.id];
            const setupStatus = setupByTask[task.id]?.status ?? task.setupStatus ?? null;
            return (
              <li key={task.id}>
                <button
                  type="button"
                  className={
                    "sidebar__task" +
                    (selected ? " sidebar__task--selected" : "") +
                    (needsAttention ? " sidebar__task--attention" : "")
                  }
                  onClick={() => setSelectedTask(task.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setMenu({ taskId: task.id, x: e.clientX, y: e.clientY });
                  }}
                >
                  <StatusDot status={dotStatus} />
                  {task.ticketKey && task.title.trim() && (
                    <span className="sidebar__task-key">{task.ticketKey}</span>
                  )}
                  <span className="sidebar__task-title">{taskName(task)}</span>
                  {needsAttention && (
                    <span
                      className="sidebar__task-attention"
                      aria-label="needs attention"
                    />
                  )}
                  {setupStatus === "running" && (
                    <span className="sidebar__task-setup sidebar__task-setup--running"
                          role="status" aria-label="setup running" title="Setup running" />
                  )}
                  {setupStatus === "failed" && (
                    <span className="sidebar__task-setup sidebar__task-setup--failed"
                          role="status" aria-label="setup failed" title="Setup failed" />
                  )}
                  {task.prNumber != null && (
                    <span className="sidebar__task-pr" title={`PR #${task.prNumber}`}>
                      PR
                    </span>
                  )}
                  {task.status === "pending" && task.blockedBy && task.blockedBy.length > 0 && (
                    <span
                      className="sidebar__task-blockers"
                      title={`Blocked by: ${task.blockedBy.map((b) => b.title ?? b.taskId).join(", ")}`}
                    >
                      ⛓ {task.blockedBy.length}
                    </span>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {filteredHiddenTasks.length > 0 && (
        <>
          <button
            type="button"
            className="sidebar__hidden-header"
            onClick={() => setHiddenCollapsed((v) => !v)}
            aria-expanded={!hiddenCollapsed}
          >
            {hiddenCollapsed ? `↑ ${filteredHiddenTasks.length} hidden` : `↓ ${filteredHiddenTasks.length} hidden`}
          </button>
          {!hiddenCollapsed && (
            <ul className="sidebar__tasks sidebar__tasks--hidden">
              {filteredHiddenTasks.map((task) => {
                const agentSession = sessionsByTask[task.id]?.find((s) => s.kind === "agent");
                const dotStatus = agentSession?.activity ?? task.status;
                const selected = task.id === selectedTaskId;
                const needsAttention = !selected && !!attentionByTask[task.id];
                const setupStatus = setupByTask[task.id]?.status ?? task.setupStatus ?? null;
                return (
                  <li key={task.id}>
                    <button
                      type="button"
                      className={
                        "sidebar__task" +
                        (selected ? " sidebar__task--selected" : "") +
                        (needsAttention ? " sidebar__task--attention" : "")
                      }
                      onClick={() => setSelectedTask(task.id)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        setHiddenMenu({ taskId: task.id, x: e.clientX, y: e.clientY });
                      }}
                    >
                      <StatusDot status={dotStatus} />
                      {task.ticketKey && task.title.trim() && (
                        <span className="sidebar__task-key">{task.ticketKey}</span>
                      )}
                      <span className="sidebar__task-title">{taskName(task)}</span>
                      {needsAttention && (
                        <span
                          className="sidebar__task-attention"
                          aria-label="needs attention"
                        />
                      )}
                      {setupStatus === "running" && (
                        <span
                          className="sidebar__task-setup sidebar__task-setup--running"
                          role="status"
                          aria-label="setup running"
                          title="Setup running"
                        />
                      )}
                      {setupStatus === "failed" && (
                        <span
                          className="sidebar__task-setup sidebar__task-setup--failed"
                          role="status"
                          aria-label="setup failed"
                          title="Setup failed"
                        />
                      )}
                      {task.prNumber != null && (
                        <span className="sidebar__task-pr" title={`PR #${task.prNumber}`}>
                          PR
                        </span>
                      )}
                      {task.status === "pending" && task.blockedBy && task.blockedBy.length > 0 && (
                        <span
                          className="sidebar__task-blockers"
                          title={`Blocked by: ${task.blockedBy.map((b) => b.title ?? b.taskId).join(", ")}`}
                        >
                          ⛓ {task.blockedBy.length}
                        </span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </>
      )}
      {showSettings && (
        <RepoSettingsModal repo={repo} onClose={() => setShowSettings(false)} />
      )}
      {menu && (
        <ContextMenu
          position={{ x: menu.x, y: menu.y }}
          onClose={() => setMenu(null)}
          items={[
            {
              label: "Hide",
              onSelect: () => hideTask(menu.taskId),
            } satisfies ContextMenuItem,
            {
              label: "Delete",
              danger: true,
              onSelect: () => setConfirmTaskId(menu.taskId),
            } satisfies ContextMenuItem,
          ]}
        />
      )}
      {hiddenMenu && (
        <ContextMenu
          position={{ x: hiddenMenu.x, y: hiddenMenu.y }}
          onClose={() => setHiddenMenu(null)}
          items={[
            {
              label: "Reopen",
              onSelect: () => reopenTask(hiddenMenu.taskId),
            } satisfies ContextMenuItem,
            {
              label: "Delete",
              danger: true,
              onSelect: () => setConfirmTaskId(hiddenMenu.taskId),
            } satisfies ContextMenuItem,
          ]}
        />
      )}
      {confirmTaskId &&
        (() => {
          const t = tasks.find((task) => task.id === confirmTaskId) || filteredHiddenTasks.find((task) => task.id === confirmTaskId);
          if (!t) return null;
          return (
            <DeleteTaskModal
              task={t}
              onCancel={() => setConfirmTaskId(null)}
              onConfirm={async (deleteBranch) => {
                await deleteTask(t.id, deleteBranch);
                setConfirmTaskId(null);
              }}
            />
          );
        })()}
    </li>
  );
}

export function Sidebar() {
  const repos = useVigieStore((state) => state.repos);
  const refresh = useVigieStore((state) => state.refresh);
  const collapsed = useVigieStore((state) => state.sidebarCollapsed);
  const setSidebarCollapsed = useVigieStore((state) => state.setSidebarCollapsed);
  const sidebarWidth = useVigieStore((state) => state.sidebarWidth);
  const [addRepoError, setAddRepoError] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleAddRepository = async () => {
    setAddRepoError(null);
    try {
      const path = await open({ directory: true });
      if (!path || Array.isArray(path)) return;
      await addRepo(path);
      await refresh();
    } catch (err) {
      setAddRepoError(err instanceof Error ? err.message : String(err));
    }
  };

  if (collapsed) {
    return (
      <aside className="sidebar sidebar--collapsed">
        <button
          type="button"
          className="icon-btn"
          aria-label="Expand sidebar"
          onClick={() => setSidebarCollapsed(false)}
        >
          &raquo;
        </button>
      </aside>
    );
  }

  return (
    <aside className="sidebar" style={{ width: sidebarWidth }}>
      <div className="sidebar__header">
        <h2 className="sidebar__heading">Repositories</h2>
        <div className="sidebar__header-actions">
          <button
            type="button"
            className="icon-btn"
            aria-label="Add repository"
            title="Add repository"
            onClick={handleAddRepository}
          >
            +
          </button>
          <button
            type="button"
            className="icon-btn"
            aria-label="Collapse sidebar"
            onClick={() => setSidebarCollapsed(true)}
          >
            &laquo;
          </button>
        </div>
      </div>

      {addRepoError && (
        <p className="sidebar__add-repo-error" role="alert">
          {addRepoError}
        </p>
      )}

      <div className="sidebar__search">
        <span className="sidebar__search-icon" aria-hidden>
          ⌕
        </span>
        <input
          type="text"
          className="sidebar__search-input"
          placeholder="Search tasks…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search tasks"
        />
      </div>

      <div className="sidebar__scroll">
        {repos.length === 0 ? (
          <p className="sidebar__empty-state">No repositories added yet.</p>
        ) : (
          <ul className="sidebar__repos">
            {repos.map((repo) => (
              <RepoSection key={repo.id} repo={repo} search={search} />
            ))}
          </ul>
        )}
      </div>
    </aside>
  );
}
