# La Vigie

A local desktop app to manage parallel **AI coding agents** and their **git worktrees** from one
window — a local alternative to hosted "cloud agent" dashboards, centered on the code/diff. Instead
of juggling 20–30 tmux windows, each **task** is a git worktree + branch: you run an agent inside it
in an embedded terminal, watch live status, review the diff, open/merge PRs, and clean up — all in
one place.

Built with **Tauri 2** (Rust) + **React/TypeScript**.

## Features

- Multi-repo sidebar; create tasks that each get their own git worktree + branch.
- Embedded terminal per agent (spawned via a real PTY) with start / stop / resume. Pluggable agent
  engines (e.g. Claude Code and others).
- Live, at-a-glance status driven by agent hooks (working / needs-attention / idle / error) + OS
  notifications.
- Local diff review (GitHub-style, syntax-highlighted) with stage & commit.
- GitHub PR integration via the `gh` CLI: create PR, status/checks, read-only comments, squash-merge.
- Collapsible sidebar, resizable panes, diff right-or-bottom.

## Prerequisites

- **Node.js** (18+) and npm
- **Rust** via [rustup](https://rustup.rs/) (stable toolchain)
- **Xcode Command Line Tools** (macOS) — `xcode-select --install`
- **[`gh` CLI](https://cli.github.com/)**, authenticated (`gh auth login`), for the PR features
- A supported AI coding agent CLI on your `PATH` for the agent terminal

See the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for full platform details.

## Build & run

```bash
npm install
npm run tauri dev        # run the app in dev (Vite dev server + Tauri window; first build is slow)
npm run tauri build      # produce a release bundle
```

Or run `./scripts/setup-worktree.sh` (also `npm run setup`) to install JS deps and warm the Rust
build in one step.

## Testing

```bash
npx vitest run                 # frontend tests
cd src-tauri && cargo test     # Rust tests
npm run build                  # frontend typecheck + build
```

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](./CONTRIBUTING.md) for the architecture overview,
project conventions, and the dev/test workflow, and [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) for
community expectations. Security issues: please read [SECURITY.md](./SECURITY.md).

## License

Licensed under the [MIT License](./LICENSE).
