import { describe, it, expect } from "vitest";
import {
  buildReport,
  truncate,
  ISSUE_LABEL,
  ISSUE_MARKER,
  ISSUE_TITLE,
  MAX_DETAIL_CHARS,
} from "./security-scan-report.mjs";

const clean = [
  { name: "Rust advisories (cargo audit)", found: false, details: "" },
  { name: "npm advisories (npm audit)", found: false, details: "" },
  { name: "Secret detection (gitleaks)", found: false, details: "" },
];

describe("buildReport", () => {
  it("reports no findings when every scanner is clean", () => {
    const r = buildReport(clean);
    expect(r.hasFindings).toBe(false);
    expect(r.title).toBe(ISSUE_TITLE);
    expect(r.body).toContain(ISSUE_MARKER);
    expect(r.body).toContain("no findings");
    // one ✅ per scanner, no ⚠️
    expect(r.body.match(/✅/g)).toHaveLength(3);
    expect(r.body).not.toContain("⚠️");
  });

  it("flags findings and fences the offending scanner's details", () => {
    const results = [
      { name: "Rust advisories (cargo audit)", found: true, details: "RUSTSEC-2024-0001: bad crate" },
      ...clean.slice(1),
    ];
    const r = buildReport(results);
    expect(r.hasFindings).toBe(true);
    expect(r.body).toContain("⚠️ Rust advisories (cargo audit)");
    expect(r.body).toContain("RUSTSEC-2024-0001: bad crate");
    // the two clean scanners still render as ✅
    expect(r.body.match(/✅/g)).toHaveLength(2);
  });

  it("appends the run link when a runUrl is given", () => {
    const r = buildReport(clean, { runUrl: "https://example.com/run/1" });
    expect(r.body).toContain("[View the workflow run](https://example.com/run/1)");
  });

  it("truncates over-long details with a notice", () => {
    const long = "x".repeat(MAX_DETAIL_CHARS + 500);
    const out = truncate(long);
    expect(out.length).toBeLessThan(long.length);
    expect(out).toContain("truncated");
    expect(out).toContain("500 more chars");
  });

  it("truncate leaves short text (trimmed) untouched", () => {
    expect(truncate("  hi  ")).toBe("hi");
  });

  it("exposes the dedup label constant", () => {
    expect(ISSUE_LABEL).toBe("security-scan");
  });
});
