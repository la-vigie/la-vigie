---
name: verify-claims
description: Pressure-test factual claims before implementation. Run after planning, before any code is written.
targets: ["*"]
claudecode:
  allowed-tools: Bash, Read, Glob, Grep, WebFetch, Skill
---

# Verify Claims: Reality-check before code

List every factual claim the spec, task description, or your own plan depends on. Verify each one. Block implementation until claims are confirmed, falsified, or explicitly flagged as unknown.

This skill exists because the most common failure mode is asserting "X exists / doesn't exist" or "Y already works" from memory or from the task description, and writing code on top of a wrong premise. Don't.

## When to invoke

- Right after `/lavigie:spec-init`, before any code is written
- Any time you catch yourself about to claim "X doesn't exist", "Y already handles this", "Z is on FRED", "deploy.sh routes this", etc.
- Auth/IdP/redirect issues: ALWAYS enumerate config-only fixes before proposing code

## Steps

1. **Extract every factual claim** the spec or plan depends on. A claim is any assertion about:
   - **Repo state** — "endpoint X exists", "helper Y lives in file Z", "tests cover W", "branch is up to date"
   - **External APIs / data sources** — "FRED has Wilshire 5000", "Pocket-ID supports custom claims"
   - **Deploy / runtime state** — "deploy.sh auto-routes", "service is healthy", "env var is set"
   - **Task-description premises** — task descriptions can be wrong; never trust them verbatim. Treat the description itself as a set of claims to verify.

2. **Verify each claim. Quote the bytes.**
   - Repo claims → LSP `findReferences` / `goToDefinition` for symbols, `Read` / `Grep` otherwise. Don't grep for symbols when LSP is available.
   - API/data claims → `curl` the actual endpoint, fetch the docs, read the schema
   - Deploy/runtime claims → run the actual check (`docker ps`, log grep, health endpoint, gh CLI for PR/branch state)
   - Do not paraphrase. Quote the line, the response body, the log fragment that proves (or disproves) the claim.

3. **For auth / IdP / redirect / OIDC issues**, before any code is recommended, enumerate the config-only fixes:
   - Admin UI toggle in the IdP
   - Env var on the service
   - Redirect-URI list, allowed-origins, scope, claim mapping
   - Recommend code only if config genuinely can't address it, and explain why.

4. **Produce a structured reality-check report.** No prose narration — three buckets:

   ```
   ## Reality check

   ✅ Verified true
   - <claim>
     evidence: <file:line | curl response | log fragment>

   ❌ Verified false
   - <claim>
     evidence: <proof of falsity>
     impact: <which part of the plan must change>

   ⚠️ Unverifiable
   - <claim>
     why: <what blocks verification>
     unblock: <what would resolve it — test deploy, ask user, etc.>
   ```

5. **Block on findings.** If any claim is false, or any unverifiable claim is load-bearing for the plan, STOP. Update the spec via `/spec-update` with the corrected reality. Do not write code on top of a bad premise.

6. **If everything verifies clean**, say so explicitly and proceed to implementation.

## Anti-patterns (do not do these)

- "I'll verify as I go" — verification is a discrete phase, not a vibe. Run it before code.
- Paraphrasing output: "the endpoint exists" is not evidence. The line of code is.
- Skipping verification for "obvious" claims. The task description is the most common source of wrong claims.
- Recommending a code fix for an auth issue without first listing the config alternatives.
