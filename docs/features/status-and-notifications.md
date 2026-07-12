# Status and notifications

## What it is

Every task shows a live status dot driven by the agent's own **hooks**, not by
scraping the terminal — La Vigie never guesses what an agent is doing from its
output. When Claude Code fires a hook (a tool call starting, a permission
prompt, the turn ending), it posts that event to a small local server, which
turns it into a status and, if you have notifications on, an OS notification
and/or a sound.

## How to use it

**The status dot** on each task in the sidebar (and the pill in the task
header) reflects what the agent is currently doing: idle, working, waiting on
you, done, or errored. It updates the moment a hook fires — no polling, no
delay from reading terminal text.

**OS notifications** fire for the events you care about — a run completing,
failing, or needing your input — each with a native system notification you
can click to jump straight back to that task. If the app window is already
focused on the task in question, the notification is suppressed so you're not
interrupted by something you're already looking at.

**Custom sounds**: open **Settings** to pick which sound plays for each event
(Completed, Failed, Awaiting input) from the bundled palette, or import your
own audio file (mp3/wav/ogg/m4a/aac/flac) to use instead. Sounds — and the mute
toggle — can also be overridden per repository from that repo's settings, so a
noisy repo can be silenced without muting everything else.

**Automute**: enable "Mute sounds while in a meeting" in Settings and La Vigie
will suppress notification *sounds* (visual notifications still appear)
whenever your Mac detects the mic or camera is actively capturing anywhere on
the system — Zoom, Meet, Teams, a Slack huddle, whatever's running. It's off
by default and macOS-only.

## How it works

Claude Code is launched with hooks configured to `POST` their payloads to
**HookBridge**, a local loopback server La Vigie starts on an ephemeral port
at app startup. Two routes matter here:

- `POST /hook/{agent_id}` — receives Claude Code's hook events
  (`UserPromptSubmit`, `PreToolUse`, `Notification`, `Stop`, `StopFailure`,
  `SubagentStart`/`SubagentStop`), maps them to a normalized status event, and
  runs them through a small state machine.
- `POST /status/{agent_id}` — receives Claude Code's status-line payload
  (model name, context remaining) for the console info shown alongside the
  terminal.

The state machine's run-state is one of `starting`, `running`, `working`,
`idle`, `needs_attention`, `error`, or `exited` (hook-capable agents like
Claude Code move between the refined states; agents without hook support just
sit at `running`). That run-state is projected onto the task's persisted
status — `working`, `needs_attention`, `idle`, or `error` — which is what the
sidebar's status dot actually renders. (Finishing a task removes it from the
sidebar rather than parking it in a terminal state, so there's no separate
"done" dot.) A background
subagent starting or stopping doesn't change the main run-state directly; it
adjusts a counter so the displayed status stays on `working` for as long as
any subagent is still in flight, even if the main loop has already gone idle.

Each status change is emitted to the frontend as an `agent_status` event. A
hook (`useAgentStatus`) listens for it, updates the task's status in the
store, and — for the events that map to a sound (idle → "completed", error →
"failed", needs_attention → "awaiting input") — decides whether to actually
play a sound and/or raise a notification, based on your mute/per-repo/event
settings. If automute is on for the relevant repo, it first asks the native
meeting probe (a read-only CoreAudio/CoreMediaIO check for "is any device
capturing audio/video right now") and skips the sound if you're in a meeting;
the check fails open, so a probe error never silently swallows an alert.

## Extending it

The status pipeline is provider-neutral: `POST /hook/{agent_id}` and
`POST /status/{agent_id}` accept plain JSON, so a different agent engine can
drive the same status dots and notifications by posting to those endpoints
instead of (or alongside) reading terminal output. See
[Agent self-management](./agent-self-management.md) for more on the HookBridge
endpoints an agent can call about itself.
