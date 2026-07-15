#!/usr/bin/env bash
#
# poll-pr-until-merged.sh — arm auto-merge, then block until a PR reaches a
# terminal state. Designed to run under Claude Code's `run_in_background`: the
# harness re-invokes the agent when this process EXITS, so the script only exits
# on an event worth waking the agent for. Transient states (checks still
# running, or green-with-auto-merge-armed that GitHub will merge on its own) do
# NOT exit — they sleep and poll again.
#
# Exit codes (each is a "wake the agent" event):
#   0  merged
#   1  a gate / required check failed  (names printed)
#   2  PR closed without merging
#   3  safety timeout reached, still pending
#   4  all checks green but the PR will NOT merge on its own
#        (auto-merge could not be armed, or review is required / changes requested)
#   5  usage / setup error (bad args, no PR, gh failure)
#   6  merge conflict with the base branch (PR unmergeable — needs rebase/resolution)
#
# The last stdout line is always machine-readable:
#   poll-result: {"outcome":"...","pr":<n>,"exit":<code>,...}
#
# Usage:
#   poll-pr-until-merged.sh [PR|URL] [--dir DIR] [--repo OWNER/REPO]
#                           [--method merge|squash|rebase] [--interval SECS]
#                           [--timeout SECS] [--no-automerge]
#
# Defaults: PR = the one for the current branch; method = merge (matches this
# project's --no-ff merge convention); interval = 30s; timeout = 2700s (45 min).

set -uo pipefail

PR=""
DIR="$PWD"
REPO=""
METHOD="merge"
METHOD_EXPLICIT=0
INTERVAL=30
TIMEOUT=2700
ARM_AUTOMERGE=1

die() { echo "poll-pr: error: $*" >&2; echo "poll-result: {\"outcome\":\"error\",\"exit\":5,\"message\":\"$*\"}"; exit 5; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dir)         DIR="${2:?}"; shift 2 ;;
    --repo)        REPO="${2:?}"; shift 2 ;;
    --method)      METHOD="${2:?}"; METHOD_EXPLICIT=1; shift 2 ;;
    --interval)    INTERVAL="${2:?}"; shift 2 ;;
    --timeout)     TIMEOUT="${2:?}"; shift 2 ;;
    --no-automerge) ARM_AUTOMERGE=0; shift ;;
    -h|--help)     awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; exit 0 ;;
    -*)            die "unknown flag: $1" ;;
    *)             PR="$1"; shift ;;
  esac
done

command -v gh >/dev/null 2>&1 || die "gh CLI not found on PATH"
command -v jq >/dev/null 2>&1 || die "jq not found on PATH"
cd "$DIR" 2>/dev/null || die "cannot cd into --dir '$DIR'"

# gh args that pin the repo when --repo is given (else gh infers it from cwd).
GH_REPO_ARGS=()
[ -n "$REPO" ] && GH_REPO_ARGS=(--repo "$REPO")

# Resolve the PR number (from the current branch if not supplied).
if [ -z "$PR" ]; then
  PR="$(gh pr view "${GH_REPO_ARGS[@]+"${GH_REPO_ARGS[@]}"}" --json number -q .number 2>/dev/null)" \
    || die "no PR found for the current branch (pass a PR number)"
fi
[ -n "$PR" ] || die "could not resolve a PR number"

# If the merge method wasn't pinned, pick one the repo actually allows — many
# repos disable merge commits (the default), which would otherwise make arming
# auto-merge fail every time. Preference order: merge, squash, rebase.
if [ "$METHOD_EXPLICIT" -eq 0 ]; then
  allowed="$(gh repo view ${REPO:+"$REPO"} --json mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed 2>/dev/null)"
  if [ -n "$allowed" ]; then
    m=""
    [ "$(jq -r '.mergeCommitAllowed'  <<<"$allowed")" = "true" ] && m="merge"
    [ -z "$m" ] && [ "$(jq -r '.squashMergeAllowed' <<<"$allowed")" = "true" ] && m="squash"
    [ -z "$m" ] && [ "$(jq -r '.rebaseMergeAllowed' <<<"$allowed")" = "true" ] && m="rebase"
    if [ -n "$m" ] && [ "$m" != "$METHOD" ]; then
      echo "poll-pr: repo disallows '$METHOD' merges — using '$m'"
      METHOD="$m"
    fi
  fi
fi

echo "poll-pr: watching PR #$PR (method=$METHOD, interval=${INTERVAL}s, timeout=${TIMEOUT}s)"

# Best-effort: arm native auto-merge so GitHub merges the instant gates go green.
# If the repo doesn't allow auto-merge this fails harmlessly — the green-but-
# won't-merge path (exit 4) then wakes the agent instead.
if [ "$ARM_AUTOMERGE" -eq 1 ]; then
  if gh pr merge "$PR" "${GH_REPO_ARGS[@]+"${GH_REPO_ARGS[@]}"}" --auto --"$METHOD" >/dev/null 2>armerr; then
    echo "poll-pr: auto-merge armed (--$METHOD)"
  else
    echo "poll-pr: could not arm auto-merge: $(tr '\n' ' ' <armerr | sed 's/  */ /g')"
  fi
  rm -f armerr
fi

# Sets of normalized states.
FAIL_STATES="FAILURE ERROR CANCELLED TIMED_OUT ACTION_REQUIRED STARTUP_FAILURE STALE"
PEND_STATES="PENDING QUEUED IN_PROGRESS EXPECTED WAITING REQUESTED"

emit() { # $1=outcome $2=exit $3=extra-json(optional)
  local extra="${3:-}"
  [ -n "$extra" ] && extra=",$extra"
  echo "poll-result: {\"outcome\":\"$1\",\"pr\":$PR,\"exit\":$2$extra}"
  exit "$2"
}

SECONDS=0
while :; do
  json="$(gh pr view "$PR" "${GH_REPO_ARGS[@]+"${GH_REPO_ARGS[@]}"}" \
    --json state,mergedAt,reviewDecision,mergeable,mergeStateStatus,autoMergeRequest,statusCheckRollup \
    2>viewerr)" || { echo "poll-pr: gh pr view failed: $(tr '\n' ' ' <viewerr)"; rm -f viewerr; sleep "$INTERVAL"; continue; }
  rm -f viewerr

  state="$(jq -r '.state // "OPEN"' <<<"$json")"
  automerge="$(jq -r 'if .autoMergeRequest then "yes" else "no" end' <<<"$json")"
  review="$(jq -r '.reviewDecision // ""' <<<"$json")"
  mergestate="$(jq -r '.mergeStateStatus // ""' <<<"$json")"
  mergeable="$(jq -r '.mergeable // ""' <<<"$json")"

  if [ "$state" = "MERGED" ]; then
    echo "poll-pr: PR #$PR merged ✔"
    emit merged 0
  fi
  if [ "$state" = "CLOSED" ]; then
    echo "poll-pr: PR #$PR was closed without merging"
    emit closed 2
  fi

  # A merge conflict with the base branch is terminal and CI-invisible: every
  # check can be green while the PR is still unmergeable, so the "all green,
  # auto-merge armed — waiting" path below would otherwise just spin until the
  # timeout and never signal the real problem. mergeStateStatus=DIRTY (and the
  # GraphQL mergeable=CONFLICTING) mean genuine conflicts; UNKNOWN is the
  # transient "GitHub is still computing mergeability" state and must NOT trip
  # this. Exit 6 wakes the agent to rebase/resolve.
  if [ "$state" = "OPEN" ] && { [ "$mergestate" = "DIRTY" ] || [ "$mergeable" = "CONFLICTING" ]; }; then
    echo "poll-pr: PR #$PR has merge conflicts with the base branch — needs a rebase/resolution"
    emit conflict 6 "\"mergeStateStatus\":\"$mergestate\",\"mergeable\":\"$mergeable\""
  fi

  # Normalize every check/status to a single upper-case state, one per line:
  # "<STATE>\t<name>".  CheckRun that isn't COMPLETED counts as PENDING.
  checks="$(jq -r '
    .statusCheckRollup[]? |
    ((if .__typename=="CheckRun"
        then (if .status=="COMPLETED" then (.conclusion // "NEUTRAL") else "PENDING" end)
        else (.state // "PENDING") end) | ascii_upcase)
    + "\t" + (.name // .context // "check")' <<<"$json")"

  failed=""; pending=""; total=0
  while IFS=$'\t' read -r st name; do
    [ -z "$st" ] && continue
    total=$((total+1))
    case " $FAIL_STATES " in *" $st "*) failed="$failed $name($st)";; esac
    case " $PEND_STATES " in *" $st "*) pending="$pending $name";; esac
  done <<<"$checks"

  if [ -n "$failed" ]; then
    echo "poll-pr: gate FAILED:$failed"
    emit failed 1 "\"failed\":\"$(echo "$failed" | sed 's/^ //;s/"/\\"/g')\""
  fi

  # All checks concluded green (at least one check exists, none pending/failed).
  if [ "$total" -gt 0 ] && [ -z "$pending" ]; then
    if [ "$automerge" = "yes" ] && [ "$review" != "REVIEW_REQUIRED" ] && [ "$review" != "CHANGES_REQUESTED" ]; then
      # Green + auto-merge armed + not blocked on review → GitHub will merge
      # imminently. Keep polling so we exit on the real "merged" event.
      echo "poll-pr: all green, auto-merge armed — waiting for GitHub to merge…"
    else
      reason="auto-merge not armed"
      [ "$review" = "REVIEW_REQUIRED" ] && reason="review required"
      [ "$review" = "CHANGES_REQUESTED" ] && reason="changes requested"
      echo "poll-pr: all checks green but PR won't merge on its own ($reason)"
      emit green-no-automerge 4 "\"reason\":\"$reason\",\"review\":\"$review\""
    fi
  else
    echo "poll-pr: t=${SECONDS}s state=$state checks: $total total,${pending:- none} pending${failed:+, FAILED}"
  fi

  if [ "$SECONDS" -ge "$TIMEOUT" ]; then
    echo "poll-pr: timeout after ${TIMEOUT}s, still not merged"
    emit timeout 3 "\"waitedSeconds\":$SECONDS"
  fi

  sleep "$INTERVAL"
done
