import { useEffect, useRef, useState } from "react";
import { finishTask, getPrStatus, openUrl, setTaskAgent, setTaskAutoApprove, setTaskModel, stopSession } from "../../api";
import type { PrStatus } from "../../api";
import { taskName } from "../../lib/taskName";
import { useVigieStore, AGENT_TAB } from "../../store";
import type { TaskStatus, TerminalSession } from "../../store";
import { useAgents } from "../../hooks/useAgents";
import { AgentModelPicker } from "../Agent/AgentModelPicker";
import { ReviewPanel } from "../Review/ReviewPanel";
import { SetupPanel } from "./SetupPanel";
import { StatusBanner } from "../StatusBanner/StatusBanner";
import { TerminalHost } from "../Terminal/TerminalHost";
import { RunStatePill } from "../Terminal/RunStatePill";
import { useTerminalFileDrop } from "../../hooks/useTerminalFileDrop";
import { PromptPicker } from "../Prompts/PromptPicker";
import { sendToAgent } from "../Diff/sendToAgent";
import "./TaskDetail.css";

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

type DisplayStatus = TaskStatus | "running";

const STATUS_LABEL: Record<DisplayStatus, string> = {
  running: "running",
  working: "working",
  needs_attention: "needs attention",
  idle: "idle",
  done: "done",
  error: "error",
  pending: "queued",
};

const STATUS_MODIFIER: Record<DisplayStatus, string> = {
  running: "running",
  working: "working",
  needs_attention: "attention",
  idle: "idle",
  done: "done",
  error: "error",
  pending: "pending",
};

export function TaskDetail() {
  const selectedTaskId = useVigieStore((state) => state.selectedTaskId);
  const tasks = useVigieStore((state) => state.tasks);
  const repos = useVigieStore((state) => state.repos);
  const refresh = useVigieStore((state) => state.refresh);
  const setSelectedTask = useVigieStore((state) => state.setSelectedTask);
  const sessionsByTask = useVigieStore((state) => state.sessionsByTask);
  const activeTabByTask = useVigieStore((state) => state.activeTabByTask);
  const startAgentSession = useVigieStore((state) => state.startAgentSession);
  const removeAgentSession = useVigieStore((state) => state.removeAgentSession);
  const addShellSession = useVigieStore((state) => state.addShellSession);
  const removeShellSession = useVigieStore((state) => state.removeShellSession);
  const setActiveTab = useVigieStore((state) => state.setActiveTab);
  const clearTaskSessions = useVigieStore((state) => state.clearTaskSessions);
  const openSettings = useVigieStore((s) => s.openSettings);

  const [showDiff, setShowDiff] = useState(true);
  // When true the Spec/Docs dock fills the whole review/side area and the
  // terminal collapses to zero width/height. In-session only (like showDiff);
  // the TerminalHost stays mounted throughout — see KEEP-ALIVE invariant.
  const [specMaximized, setSpecMaximized] = useState(false);
  const [diffPosition, setDiffPosition] = useState<"right" | "bottom">(
    () => (localStorage.getItem("vigie.diffPosition") === "bottom" ? "bottom" : "right")
  );
  const [diffWidth, setDiffWidth] = useState<number>(
    () => Number(localStorage.getItem("vigie.diffWidth")) || 480
  );
  const [diffHeight, setDiffHeight] = useState<number>(
    () => Number(localStorage.getItem("vigie.diffHeight")) || 240
  );
  const [showFinishConfirm, setShowFinishConfirm] = useState(false);
  const [finishError, setFinishError] = useState<string | null>(null);
  const [pr, setPr] = useState<PrStatus | null>(null);

  // Agent picker state: available agents come from the shared useAgents() hook
  // (also consumed by AgentModelPicker, so a single list_agents IPC per mount).
  // agentOverride gives instant select feedback before the task default updates.
  const { agents } = useAgents();
  const [agentOverride, setAgentOverride] = useState<string | null>(null);
  // undefined = no override (use task default); null = user explicitly chose no model.
  const [modelOverride, setModelOverride] = useState<string | null | undefined>(undefined);

  const bodyRef = useRef<HTMLDivElement>(null);
  const terminalPaneRef = useRef<HTMLDivElement>(null);
  const isDropActive = useTerminalFileDrop(terminalPaneRef);

  useEffect(() => {
    localStorage.setItem("vigie.diffPosition", diffPosition);
  }, [diffPosition]);

  useEffect(() => {
    localStorage.setItem("vigie.diffWidth", String(diffWidth));
  }, [diffWidth]);

  useEffect(() => {
    localStorage.setItem("vigie.diffHeight", String(diffHeight));
  }, [diffHeight]);

  // Reset local agent/model overrides whenever the selected task changes.
  useEffect(() => {
    setAgentOverride(null);
    setModelOverride(undefined);
  }, [selectedTaskId]);

  useEffect(() => {
    if (!showFinishConfirm || !selectedTaskId) {
      setPr(null);
      return;
    }
    let cancelled = false;
    getPrStatus(selectedTaskId).then((result) => {
      if (!cancelled) setPr(result);
    }).catch(() => {
      if (!cancelled) setPr(null);
    });
    return () => { cancelled = true; };
  }, [showFinishConfirm, selectedTaskId]);

  const task = tasks.find((t) => t.id === selectedTaskId);
  // A queued task (status "pending") has no worktree and no agent yet — it's
  // waiting on a dependency to merge. Render a placeholder instead of the
  // agent Start/Resume controls and hide finish/Open-PR actions, but never
  // early-return: <TerminalHost/> below must keep rendering unconditionally
  // (KEEP-ALIVE — see class-level doc comment near its render site).
  const isPending = task?.status === "pending";

  // Derive the agent/model picker selection for the current task.
  const repoForTask = task ? repos.find((r) => r.id === task.repoId) : undefined;
  const selectedAgentName = agentOverride ?? task?.agent ?? repoForTask?.defaultAgent ?? "claude";
  const selectedAgent = agents.find((a) => a.name === selectedAgentName);
  const selectedModel = modelOverride !== undefined ? modelOverride : (task?.model ?? null);

  // Tab strip session data (derived per task)
  const sessions = task ? (sessionsByTask[task.id] ?? []) : [];
  const activeTab = task ? (activeTabByTask[task.id] ?? AGENT_TAB) : AGENT_TAB;
  const agentSession = sessions.find((s) => s.kind === "agent");
  const shellSessions = sessions.filter((s) => s.kind === "shell");

  const dotClass = (s?: TerminalSession) =>
    "terminal-tab__dot" + (s && s.status !== "exited" ? " terminal-tab__dot--live" : "");

  const handleCloseShell = async (s: TerminalSession) => {
    if (s.backendId) await stopSession(s.backendId).catch(() => {});
    if (task) removeShellSession(task.id, s.localId);
  };

  const agentInfo = agentSession;
  const isAgentActive =
    agentInfo?.status === "starting" || agentInfo?.status === "running";

  const displayStatus: DisplayStatus = isAgentActive
    ? "running"
    : (task?.status ?? "idle");

  const handleStop = async () => {
    if (agentInfo?.backendId) {
      await stopSession(agentInfo.backendId);
    }
    if (task) removeAgentSession(task.id);
  };

  const handleFinish = async (mode: "keep" | "discard" | "merge") => {
    if (!task) return;
    setFinishError(null);
    try {
      const sessions = sessionsByTask[task.id] ?? [];
      await Promise.all(
        sessions.filter((s) => s.backendId).map((s) => stopSession(s.backendId!).catch(() => {})),
      );
      clearTaskSessions(task.id);
      await finishTask(task.id, mode);
      setShowFinishConfirm(false);
      setSelectedTask(null);
      await refresh();
    } catch (err) {
      setFinishError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleDividerMouseDown = () => {
    const onMouseMove = (e: MouseEvent) => {
      const bodyRect = bodyRef.current?.getBoundingClientRect();
      if (!bodyRect) return;
      if (diffPosition === "right") {
        const newWidth = clamp(
          bodyRect.right - e.clientX,
          240,
          bodyRect.width - 200,
        );
        setDiffWidth(newWidth);
      } else {
        const newHeight = clamp(
          bodyRect.bottom - e.clientY,
          120,
          bodyRect.height - 120,
        );
        setDiffHeight(newHeight);
      }
    };
    const onMouseUp = () => {
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  };

  const diffAreaStyle =
    diffPosition === "right"
      ? { width: diffWidth }
      : { height: diffHeight };

  return (
    <main className="task-detail">
      {task ? (
        <header className="task-detail__header">
          <div className="task-detail__title-row">
            <div className="task-detail__title-main">
              {task.ticketKey && task.title.trim() && (
                <span className="task-detail__ticket" title="Ticket ID">{task.ticketKey}</span>
              )}
              <h2 className="task-detail__title">{taskName(task)}</h2>
              <span className="task-detail__branch" title={task.branch}>
                <span className="task-detail__branch-icon" aria-hidden>⎇</span>
                <span className="task-detail__branch-value">{task.branch}</span>
              </span>
              <span className="task-detail__based">
                based on <span className="task-detail__based-value">{task.baseBranch}</span>
              </span>
              <span
                className={
                  "state-badge state-badge--" + STATUS_MODIFIER[displayStatus]
                }
              >
                {STATUS_LABEL[displayStatus]}
              </span>
            </div>

            {!isPending && (
              <div className="task-detail__actions" data-testid="task-actions">
                {showFinishConfirm ? (
                  <div className="task-detail__finish-confirm">
                    {pr?.state === "OPEN" && (
                      <button
                        type="button"
                        className="btn"
                        onClick={() => handleFinish("merge")}
                      >
                        Merge PR &amp; finish
                      </button>
                    )}
                    <button type="button" className="btn" onClick={() => handleFinish("keep")}>
                      Keep branch
                    </button>
                    <button
                      type="button"
                      className="btn btn--danger"
                      onClick={() => handleFinish("discard")}
                    >
                      Discard branch
                    </button>
                    <button
                      type="button"
                      className="btn btn--ghost"
                      onClick={() => {
                        setShowFinishConfirm(false);
                        setFinishError(null);
                      }}
                    >
                      Cancel
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    className="btn"
                    onClick={() => setShowFinishConfirm(true)}
                  >
                    Finish task
                  </button>
                )}

                <button
                  type="button"
                  className="btn btn--primary"
                  disabled={!task.prUrl}
                  title={task.prUrl ? "Open pull request in browser" : "No pull request yet"}
                  onClick={() => {
                    if (task.prUrl) openUrl(task.prUrl).catch(() => {});
                  }}
                >
                  Open PR <span aria-hidden>↗</span>
                </button>
              </div>
            )}
          </div>

          <div className="task-detail__path" title={task.worktreePath}>
            {task.worktreePath}
          </div>

          {task && <SetupPanel taskId={task.id} />}

          {finishError && (
            <p className="task-detail__finish-error" role="alert">
              {finishError}
            </p>
          )}
        </header>
      ) : (
        <div className="task-detail__empty">
          <p>Select a task</p>
        </div>
      )}

      {/* TerminalHost is rendered OUTSIDE the task/no-task conditional so it
          stays mounted (same DOM position) across all selection changes —
          this preserves the keep-alive guarantee for every running PTY. */}
      <div
        ref={bodyRef}
        className={
          "task-detail__body" +
          (diffPosition === "bottom" ? " task-detail__body--bottom" : "") +
          (specMaximized ? " task-detail__body--spec-max" : "")
        }
      >
        <section className="task-detail__terminal-area">
          {task && (
            <div className="terminal-pane__header">
              <div className="terminal-pane__tabs" role="tablist">
                <div
                  role="tab"
                  tabIndex={0}
                  aria-selected={activeTab === AGENT_TAB}
                  className={"terminal-tab" + (activeTab === AGENT_TAB ? " terminal-tab--active" : "")}
                  onClick={() => setActiveTab(task.id, AGENT_TAB)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      if (e.key === " ") e.preventDefault();
                      setActiveTab(task.id, AGENT_TAB);
                    }
                  }}
                >
                  <span className="terminal-tab__icon" aria-hidden>{(agentSession?.title ?? "Claude").charAt(0).toUpperCase()}</span>
                  <span className="terminal-tab__label">{agentSession?.title ?? "Claude"}</span>
                  <span className={dotClass(agentSession)} aria-hidden />
                </div>
                {shellSessions.map((s) => (
                  <div
                    key={s.localId}
                    role="tab"
                    tabIndex={0}
                    aria-selected={activeTab === s.localId}
                    className={"terminal-tab" + (activeTab === s.localId ? " terminal-tab--active" : "")}
                    onClick={() => setActiveTab(task.id, s.localId)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        if (e.key === " ") e.preventDefault();
                        setActiveTab(task.id, s.localId);
                      }
                    }}
                  >
                    <span className="terminal-tab__icon" aria-hidden>$</span>
                    <span className="terminal-tab__label">{s.title}</span>
                    <span className={dotClass(s)} aria-hidden />
                    <button
                      type="button"
                      aria-label={`Close ${s.title}`}
                      className="terminal-tab__close"
                      onClick={(e) => { e.stopPropagation(); handleCloseShell(s); }}
                    >×</button>
                  </div>
                ))}
                <button
                  type="button"
                  className="terminal-tab terminal-tab--add"
                  aria-label="New terminal"
                  onClick={() => addShellSession(task.id)}
                >+</button>
              </div>
              <div className="terminal-pane__header-actions">
                <PromptPicker
                  onSelect={(body) => { void sendToAgent(task.id, body); }}
                  onManage={openSettings}
                />
              </div>
            </div>
          )}
          <div
            ref={terminalPaneRef}
            className={"terminal-pane__body" + (isDropActive ? " terminal-pane__body--drop-active" : "")}
          >
            {task && activeTab === AGENT_TAB && !isAgentActive && (
              isPending ? (
                <div className="task-detail__queued" role="status">
                  <p className="task-detail__queued-title">Queued</p>
                  <p className="task-detail__queued-hint">
                    This task will start automatically when its dependency merges.
                  </p>
                </div>
              ) : (
                <div className="terminal-pane__placeholder">
                  <p>Agent not running.</p>
                  <div className="terminal-pane__placeholder-actions">
                    <AgentModelPicker
                      agent={selectedAgentName}
                      model={selectedModel}
                      onChange={(a, m) => {
                        setAgentOverride(a);
                        setTaskAgent(task.id, a);
                        setModelOverride(m);
                        setTaskModel(task.id, m);
                      }}
                    />
                    <label className="agent-auto-approve">
                      <span>Auto-approve</span>
                      <select
                        aria-label="Auto-approve for this task"
                        value={
                          task.autoApprove == null ? "inherit" : task.autoApprove ? "on" : "off"
                        }
                        onChange={(e) => {
                          const v =
                            e.target.value === "inherit" ? null : e.target.value === "on";
                          setTaskAutoApprove(task.id, v).then(refresh);
                        }}
                      >
                        <option value="inherit">Inherit from repo</option>
                        <option value="on">On</option>
                        <option value="off">Off</option>
                      </select>
                    </label>
                    <button
                      type="button"
                      className="btn btn--primary"
                      onClick={() =>
                        startAgentSession(task.id, false, selectedAgent
                          ? { label: selectedAgent.displayName, lifecycle: selectedAgent.status === "lifecycle" }
                          : undefined)
                      }
                    >
                      Start agent
                    </button>
                    <button
                      type="button"
                      className="btn btn--ghost"
                      disabled={!selectedAgent || selectedAgent.resumeArgs.length === 0}
                      onClick={() =>
                        startAgentSession(task.id, true, selectedAgent
                          ? { label: selectedAgent.displayName, lifecycle: selectedAgent.status === "lifecycle" }
                          : undefined)
                      }
                    >
                      Resume
                    </button>
                  </div>
                </div>
              )
            )}
            {task && activeTab === AGENT_TAB && isAgentActive && agentInfo && (
              <RunStatePill status={agentInfo.status} activity={agentInfo.activity} lifecycle={agentInfo.lifecycle} onStop={handleStop} />
            )}
            {/* KEEP-ALIVE: <TerminalHost/> must never unmount/remount while an
                agent runs — the live PTY lives inside it, so remounting kills it.
                Keep this at a stable DOM position; swap content around it (tabs,
                pill, placeholder above; diff/spec panes as siblings), never wrap
                or conditionally render it. Any layout change here needs a
                DOM-identity (keep-alive) test. */}
            <TerminalHost />
          </div>
          {task && <StatusBanner task={task} />}
        </section>
        {task && showDiff ? (
          <>
            {!specMaximized && (
              <div
                className={"resize-handle " + (diffPosition === "right" ? "resize-handle--x" : "resize-handle--y")}
                role="separator"
                aria-orientation={diffPosition === "right" ? "vertical" : "horizontal"}
                aria-label="Resize diff panel"
                onMouseDown={handleDividerMouseDown}
              />
            )}
            <div className="task-detail__diff-area" style={specMaximized ? undefined : diffAreaStyle}>
              <ReviewPanel
                taskId={task.id}
                showDiff={showDiff}
                onToggleDiff={() => setShowDiff((v) => !v)}
                diffPosition={diffPosition}
                onSetDiffPosition={setDiffPosition}
                specMaximized={specMaximized}
                onToggleSpecMaximize={() => setSpecMaximized((v) => !v)}
              />
            </div>
          </>
        ) : task ? (
          <button type="button" className="task-detail__changes-rail" aria-label="Show changes" onClick={() => setShowDiff(true)}>
            Changes
          </button>
        ) : null}
      </div>
    </main>
  );
}
