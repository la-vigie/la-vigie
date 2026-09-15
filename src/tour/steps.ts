// The v1 product-tour step list, in the required order:
//   welcome → core loop → schedules → remote → finish.
//
// `anchor` is a data-tour key on a live DOM node (or null for a centered card).
// `reveal` is a non-destructive UI action key the overlay runs to surface an
// anchor that isn't currently mounted (see REVEAL_ACTIONS in the overlay). If an
// anchor still can't be resolved the step degrades to a centered card — the tour
// never fabricates demo state (no auto repo/worktree/PTY).

import type { TourStep } from "./tourMachine";

export const TOUR_STEPS: TourStep[] = [
  {
    id: "welcome",
    title: "Welcome to La Vigie",
    body:
      "Manage parallel AI agents and their git worktrees from one window. This quick tour points at the real controls — you can leave anytime with Skip.",
    anchor: null,
  },
  {
    id: "add-repo",
    section: "core",
    title: "1. Add a repository",
    body:
      "Start here: add a git repo to manage. Each task you create gets its own worktree under it, so agents never step on each other.",
    anchor: "add-repo",
    placement: "bottom",
  },
  {
    id: "new-task",
    section: "core",
    title: "2. Create a task",
    body:
      "A task is a git worktree + branch — an isolated place for one agent to work. Create one per unit of work.",
    anchor: "new-task",
    placement: "right",
  },
  {
    id: "start-agent",
    section: "core",
    title: "3. Start an agent",
    body:
      "Launch Claude (or another engine) inside the task's worktree. The agent is an ephemeral session you can start, stop, and resume.",
    anchor: "start-agent",
    placement: "auto",
    reveal: "select-first-task",
  },
  {
    id: "terminal",
    section: "core",
    title: "4. Watch it work",
    body:
      "This is the live terminal — the agent's real PTY. The status dot is driven by Claude Code hooks (not scraped output), so it tells you honestly when the agent needs you.",
    anchor: "terminal",
    placement: "auto",
    reveal: "select-first-task",
  },
  {
    id: "review",
    section: "core",
    title: "5. Review the changes",
    body:
      "Glance at the diff as the agent edits — uncommitted work or the whole branch vs. its base. When you're ready, Finish task opens a pull request.",
    anchor: "review",
    placement: "left",
    reveal: "select-first-task",
  },
  {
    id: "schedules",
    section: "schedules",
    title: "Automate dispatch",
    body:
      "Open a repo's settings to schedule work: recurring cron dispatch, or one-shot deferred tasks that launch in N hours (also the ‘Start later’ checkbox when creating a task).",
    anchor: "schedules",
    placement: "right",
  },
  {
    id: "remote",
    section: "remote",
    title: "Watch from your phone",
    body:
      "Open Settings → Remote to enable the mobile server and scan the QR code from your phone — then watch agents and reply on the go.",
    anchor: "remote",
    placement: "bottom",
  },
  {
    id: "finish",
    title: "You're all set",
    body:
      "That's the core loop. Re-run this tour anytime from the ? button in the top bar. Happy shipping!",
    anchor: null,
  },
];
