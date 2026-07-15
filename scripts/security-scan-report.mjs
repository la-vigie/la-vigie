// Pure, dependency-free core of the scheduled security scan (TASK-71).
//
// It turns three scanner verdicts (cargo audit / npm audit / gitleaks) into the
// title + body of a single deduplicated GitHub issue. It touches NO fs and NO
// network, so it is unit-tested headlessly (`npx vitest run`) and imported by
// the thin runner (`security-scan.mjs`) that the GitHub Action shells out to.
//
// Dedup contract: the workflow finds the previous issue by the ISSUE_LABEL, so
// the title is stable; the marker lets a human (or future tooling) recognise a
// scan-authored issue body.

export const ISSUE_LABEL = "security-scan";
export const ISSUE_MARKER = "<!-- security-scan:sticky -->";
export const ISSUE_TITLE = "🔒 Scheduled security scan findings";

// GitHub issue bodies cap at 65536 chars; keep each scanner's detail well under
// that so three sections plus scaffolding always fit.
export const MAX_DETAIL_CHARS = 15000;

export function truncate(text, max = MAX_DETAIL_CHARS) {
  const t = (text || "").trim();
  if (t.length <= max) return t;
  return `${t.slice(0, max)}\n… (truncated ${t.length - max} more chars)`;
}

// results: [{ name, found, details }] — one entry per scanner, in display order.
export function buildReport(results, opts = {}) {
  const { runUrl } = opts;
  const hasFindings = results.some((r) => r.found);
  const lines = [ISSUE_MARKER, ""];
  lines.push(
    hasFindings
      ? "The scheduled security scan found issues that need attention."
      : "The scheduled security scan completed with no findings.",
  );
  lines.push("");
  for (const r of results) {
    const icon = r.found ? "⚠️" : "✅";
    lines.push(`### ${icon} ${r.name}`);
    lines.push("");
    if (r.found) {
      lines.push("```");
      lines.push(truncate(r.details) || "(no detail captured)");
      lines.push("```");
    } else {
      lines.push("No findings.");
    }
    lines.push("");
  }
  if (runUrl) {
    lines.push("---");
    lines.push(`[View the workflow run](${runUrl})`);
  }
  return { hasFindings, title: ISSUE_TITLE, body: lines.join("\n") };
}
