# GitHub pull requests

## What it is

La Vigie can open, check, and merge pull requests for a task without storing any
credentials of its own. PR support is built entirely on top of the [`gh` CLI](https://cli.github.com/) —
La Vigie shells out to `gh` for every GitHub call, so there's no token to paste into
the app and no OAuth flow to grant. You authenticate `gh` yourself, once, outside
the app, and La Vigie picks up that session.

## How to use it

1. **Authenticate `gh`** on your machine, if you haven't already:
   ```sh
   gh auth login
   ```
2. Open a task's **Review** panel and switch to the **PR** tab. If `gh` isn't
   installed or isn't authenticated, the panel tells you so (with a **Refresh**
   button to re-check) instead of showing PR content.
3. **Create a PR.** If the task's branch has no open PR yet, the tab shows a
   create form pre-filled with the task's title. Fill in a title/body, optionally
   check **Create as draft**, and click **Create**. La Vigie pushes the branch to
   `origin` and opens the PR via `gh pr create`.
4. **Read status and checks.** Once a PR exists, the tab shows its state (open /
   merged / closed), draft flag, mergeable status, review decision, and the
   per-check status rollup (success/failure/pending/neutral) — all pulled live
   from `gh pr view`. Use **Refresh** to re-fetch, or **Open in browser** to jump
   to the PR on GitHub.
5. **Read comments.** Issue comments, review comments, and inline review
   comments are listed underneath, each tagged with its author, kind, and —
   for inline comments — the `file:line` it was left on. This view is read-only;
   there's no reply box.
6. **Merge.** Merging happens from the task's **Finish** flow, not the PR tab:
   open the finish confirmation and, if the PR is open, a **Merge PR & finish**
   button appears alongside the usual **Keep branch** / **Discard** options.
   Clicking it squash-merges the PR (`gh pr merge --squash`) and then removes
   the task's worktree.

## How it works

Every GitHub (and git) call La Vigie makes goes through `gh`/`git` invoked with an
argv vector — never through a shell. That means a task title, PR title/body, or
branch name can contain arbitrary characters (quotes, `;`, `$(...)`, etc.) without
any risk of shell command injection, because there's no shell parsing those
arguments in the first place.

On the frontend, PR titles, bodies, and comments are rendered as plain React text
(never through `dangerouslySetInnerHTML`), so PR content from GitHub can't inject
HTML or script into the app.

`gh` itself is a prerequisite: it must be installed and authenticated
(`gh auth login`) for any of this to work. La Vigie checks both conditions before
showing PR content and reports which one is missing if either fails.
