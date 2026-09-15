// Pure client-side validator for a repo's auto-routing policy JSON.
//
// Mirrors the Rust `agent::routing::RoutingPolicy` shape so the settings UI can
// give an inline error before hitting the backend (which is the authority and
// re-validates). Returns `null` when `raw` is valid, else a human-readable
// message naming the first problem. An empty/blank string is valid (⇒ clear the
// policy). Kept dependency-free and side-effect-free so it is trivially unit-tested.

/**
 * Whether a stored policy string parses to an enabled policy. Used by the New
 * Task form to decide whether to offer (and default to) auto-routing — when true
 * the form sends no explicit agent so the backend router fires. Lenient: any
 * parse problem ⇒ false (routing off).
 */
export function routingPolicyEnabled(raw?: string | null): boolean {
  if (!raw || raw.trim() === "") return false;
  try {
    const parsed: unknown = JSON.parse(raw);
    return (
      typeof parsed === "object" &&
      parsed !== null &&
      !Array.isArray(parsed) &&
      (parsed as Record<string, unknown>).enabled === true
    );
  } catch {
    return false;
  }
}

const DIFFICULTIES = ["easy", "medium", "hard"] as const;
const KINDS = ["refactor", "greenfield", "debug", "ui", "docs", "other"] as const;

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** An optional field is "present" only when it's neither undefined nor null.
 * `null` is treated as absent, so a canonical `{"difficulty": null, ...}` (or an
 * older stored policy) validates the same as omitting the field. */
function isPresent(v: unknown): boolean {
  return v !== undefined && v !== null;
}

/** Reject keys the backend's `deny_unknown_fields` would reject, so a typo like
 * `rulez` is flagged here rather than silently dropped on save. */
function rejectUnknownKeys(
  o: Record<string, unknown>,
  allowed: readonly string[],
  where: string,
): string | null {
  for (const key of Object.keys(o)) {
    if (!allowed.includes(key)) {
      return `${where} has an unknown field ${JSON.stringify(key)} (allowed: ${allowed.join(", ")})`;
    }
  }
  return null;
}

function validateTarget(t: unknown, where: string): string | null {
  if (!isPlainObject(t)) return `${where} must be an object`;
  const unknown = rejectUnknownKeys(t, ["agent", "model"], where);
  if (unknown) return unknown;
  if (typeof t.agent !== "string" || t.agent.trim() === "") {
    return `${where}.agent must be a non-empty string`;
  }
  if (isPresent(t.model) && typeof t.model !== "string") {
    return `${where}.model must be a string`;
  }
  return null;
}

function validateEnumArray(
  v: unknown,
  allowed: readonly string[],
  where: string,
): string | null {
  if (!Array.isArray(v)) return `${where} must be an array`;
  if (v.length === 0) return `${where} must not be empty`;
  for (const item of v) {
    if (typeof item !== "string" || !allowed.includes(item)) {
      return `${where} has an invalid value ${JSON.stringify(item)} (allowed: ${allowed.join(", ")})`;
    }
  }
  return null;
}

function validateCondition(w: unknown, where: string): string | null {
  if (!isPlainObject(w)) return `${where} must be an object`;
  const unknown = rejectUnknownKeys(w, ["difficulty", "kind", "titleContains"], where);
  if (unknown) return unknown;
  if (isPresent(w.difficulty)) {
    const e = validateEnumArray(w.difficulty, DIFFICULTIES, `${where}.difficulty`);
    if (e) return e;
  }
  if (isPresent(w.kind)) {
    const e = validateEnumArray(w.kind, KINDS, `${where}.kind`);
    if (e) return e;
  }
  if (isPresent(w.titleContains)) {
    if (typeof w.titleContains !== "string") {
      return `${where}.titleContains must be a string`;
    }
    // A blank needle is an inert clause backend-side (never a wildcard); reject
    // it here so the user doesn't write a filter that quietly does nothing.
    if (w.titleContains.trim() === "") {
      return `${where}.titleContains must not be blank`;
    }
  }
  return null;
}

/**
 * Validate a routing-policy JSON string. `null` ⇒ valid. A blank string is
 * treated as "clear the policy" and is valid.
 */
export function validateRoutingPolicy(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch (e) {
    return `Not valid JSON: ${e instanceof Error ? e.message : String(e)}`;
  }

  if (!isPlainObject(parsed)) return "Policy must be a JSON object";
  const unknownTop = rejectUnknownKeys(parsed, ["enabled", "rules", "fallback"], "policy");
  if (unknownTop) return unknownTop;
  if (typeof parsed.enabled !== "boolean") {
    return "`enabled` is required and must be true or false";
  }

  if (parsed.rules !== undefined) {
    if (!Array.isArray(parsed.rules)) return "`rules` must be an array";
    for (let i = 0; i < parsed.rules.length; i++) {
      const rule = parsed.rules[i];
      const at = `rules[${i}]`;
      if (!isPlainObject(rule)) return `${at} must be an object`;
      const unknownRule = rejectUnknownKeys(rule, ["when", "target"], at);
      if (unknownRule) return unknownRule;
      if (rule.when !== undefined) {
        const e = validateCondition(rule.when, `${at}.when`);
        if (e) return e;
      }
      const e = validateTarget(rule.target, `${at}.target`);
      if (e) return e;
    }
  }

  if (parsed.fallback !== undefined && parsed.fallback !== null) {
    const e = validateTarget(parsed.fallback, "fallback");
    if (e) return e;
  }

  return null;
}
