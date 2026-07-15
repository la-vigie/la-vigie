// Thin runner (untested glue) for the scheduled security scan (TASK-71).
//
// Each scanner step in .github/workflows/security-scan.yml writes:
//   <name>.status  — the single word "found" or "clean"
//   <name>.txt     — the scanner's captured (redacted) output
// This runner reads those, calls the pure buildReport(), and hands the workflow
// a body file + a has_findings output. It touches fs + env only; all decision
// logic lives in the unit-tested security-scan-report.mjs.

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { buildReport } from "./security-scan-report.mjs";

function readScanner(slug, label) {
  const statusFile = `${slug}.status`;
  const detailFile = `${slug}.txt`;
  // No status file ⇒ the scanner never reported ⇒ treat as a finding so a
  // crashed/absent scanner surfaces instead of silently passing.
  const found = existsSync(statusFile)
    ? readFileSync(statusFile, "utf8").trim() === "found"
    : true;
  let details = existsSync(detailFile) ? readFileSync(detailFile, "utf8") : "";
  if (!existsSync(statusFile)) {
    details =
      details ||
      `Scanner "${slug}" produced no status file — treated as a finding so a broken scan is never silent.`;
  }
  return { name: label, found, details };
}

const results = [
  readScanner("cargo-audit", "Rust advisories (cargo audit)"),
  readScanner("npm-audit", "npm advisories (npm audit)"),
  readScanner("gitleaks", "Secret detection (gitleaks)"),
];

const { hasFindings, body } = buildReport(results, {
  runUrl: process.env.RUN_URL || "",
});

writeFileSync("security-issue-body.md", body);
if (process.env.GITHUB_OUTPUT) {
  writeFileSync(process.env.GITHUB_OUTPUT, `has_findings=${hasFindings}\n`, {
    flag: "a",
  });
}
console.log(`has_findings=${hasFindings}`);
