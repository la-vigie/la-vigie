# La Vigie documentation

La Vigie is a local desktop app for managing parallel AI coding agents and their git
worktrees from one window. This is the documentation index — start with the Getting
Started tour, then dig into whichever feature you need.

## Getting started

- [Getting Started](./getting-started.md) — a screenshot walkthrough of the core loop:
  create a task, run an agent in it, review the diff, and steer the agent.

## Features

- [Tasks and worktrees](./features/tasks-and-worktrees.md) — the multi-repo sidebar and
  the durable Task = git worktree + branch model: create, finish, delete, and hide tasks.
- [Agents and the terminal](./features/agents-and-terminal.md) — the embedded PTY
  terminal per task, pluggable agent engines and models, start/stop/resume, shell tabs,
  and the reusable prompt library.
- [Status and notifications](./features/status-and-notifications.md) — live, hook-driven
  agent status; status dots; OS notifications; custom sounds; and automute while you're
  in a meeting.
- [Diff and review](./features/diff-and-review.md) — the in-app diff (uncommitted vs.
  compared-to-base), stage & commit, inline review comments sent to the agent, and the
  Spec / Docs dock.
- [GitHub pull requests](./features/github-prs.md) — PR integration through the `gh` CLI:
  create a PR, read status/checks and comments, and squash-merge.
- [Remote control](./features/remote-control.md) — reach the cockpit from your phone over
  your tailnet: the Tailscale-served web client, QR pairing, read-and-reply, and
  worktree-less concierge sessions.
- [MCP server](./features/mcp-server.md) — the in-process MCP server that lets an agent
  drive the cockpit programmatically: dispatch tasks and inspect work in flight.
- [Agent self-management](./features/agent-self-management.md) — how an agent calls back
  into La Vigie over HookBridge to rename or tear down its own task, and how to extend
  that seam.

## For contributors

- [Mutation testing](./mutation-testing.md) — what we mutate (and deliberately don't),
  how to run it, and the advisory → gating rollout.
- [CONTRIBUTING](../CONTRIBUTING.md) — architecture overview, project conventions, and the
  dev/test workflow.
- [AGENTS.md](../AGENTS.md) — the project guide for AI agents working in this codebase
  (architecture, invariants, and conventions).
