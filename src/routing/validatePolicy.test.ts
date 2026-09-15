import { describe, it, expect } from "vitest";
import { validateRoutingPolicy } from "./validatePolicy";

describe("validateRoutingPolicy", () => {
  it("treats blank / whitespace as valid (clears the policy)", () => {
    expect(validateRoutingPolicy("")).toBeNull();
    expect(validateRoutingPolicy("   \n ")).toBeNull();
  });

  it("accepts a full, well-formed policy", () => {
    const raw = JSON.stringify({
      enabled: true,
      rules: [
        { when: { difficulty: ["hard"], kind: ["debug"] }, target: { agent: "claude", model: "opus" } },
        { when: { titleContains: "css" }, target: { agent: "cursor" } },
        { target: { agent: "codex" } },
      ],
      fallback: { agent: "codex", model: "gpt-5.5" },
    });
    expect(validateRoutingPolicy(raw)).toBeNull();
  });

  it("accepts a minimal disabled policy", () => {
    expect(validateRoutingPolicy('{"enabled": false}')).toBeNull();
  });

  it("rejects invalid JSON", () => {
    expect(validateRoutingPolicy("{not json")).toMatch(/not valid json/i);
  });

  it("rejects a non-object top level", () => {
    expect(validateRoutingPolicy("[]")).toMatch(/must be a JSON object/);
    expect(validateRoutingPolicy("42")).toMatch(/must be a JSON object/);
  });

  it("requires a boolean `enabled`", () => {
    expect(validateRoutingPolicy('{"rules": []}')).toMatch(/enabled/);
    expect(validateRoutingPolicy('{"enabled": "yes"}')).toMatch(/enabled/);
  });

  it("rejects a non-array rules", () => {
    expect(validateRoutingPolicy('{"enabled": true, "rules": {}}')).toMatch(/`rules` must be an array/);
  });

  it("requires a rule target with a non-empty agent", () => {
    expect(
      validateRoutingPolicy('{"enabled": true, "rules": [{ "when": {} }]}'),
    ).toMatch(/rules\[0\]\.target must be an object/);
    expect(
      validateRoutingPolicy('{"enabled": true, "rules": [{ "target": { "agent": "" } }]}'),
    ).toMatch(/rules\[0\]\.target\.agent must be a non-empty string/);
  });

  it("rejects an unknown difficulty / kind value", () => {
    expect(
      validateRoutingPolicy(
        '{"enabled": true, "rules": [{ "when": { "difficulty": ["epic"] }, "target": { "agent": "claude" } }]}',
      ),
    ).toMatch(/difficulty has an invalid value/);
    expect(
      validateRoutingPolicy(
        '{"enabled": true, "rules": [{ "when": { "kind": ["chore"] }, "target": { "agent": "claude" } }]}',
      ),
    ).toMatch(/kind has an invalid value/);
  });

  it("rejects a non-string titleContains", () => {
    expect(
      validateRoutingPolicy(
        '{"enabled": true, "rules": [{ "when": { "titleContains": 5 }, "target": { "agent": "claude" } }]}',
      ),
    ).toMatch(/titleContains must be a string/);
  });

  it("validates the fallback target when present", () => {
    expect(
      validateRoutingPolicy('{"enabled": true, "fallback": { "model": "opus" }}'),
    ).toMatch(/fallback\.agent must be a non-empty string/);
  });

  it("allows a null fallback", () => {
    expect(validateRoutingPolicy('{"enabled": true, "fallback": null}')).toBeNull();
  });

  it("rejects unknown top-level keys (typo guard, matches backend deny_unknown_fields)", () => {
    expect(validateRoutingPolicy('{"enabled": true, "rulez": []}')).toMatch(
      /policy has an unknown field "rulez"/,
    );
  });

  it("rejects unknown keys inside a rule / condition / target", () => {
    expect(
      validateRoutingPolicy('{"enabled": true, "rules": [{ "target": { "agent": "codex" }, "wen": {} }]}'),
    ).toMatch(/rules\[0\] has an unknown field "wen"/);
    expect(
      validateRoutingPolicy(
        '{"enabled": true, "rules": [{ "when": { "titleCncludes": "x" }, "target": { "agent": "codex" } }]}',
      ),
    ).toMatch(/unknown field "titleCncludes"/);
    expect(
      validateRoutingPolicy(
        '{"enabled": true, "rules": [{ "target": { "agent": "codex", "modle": "x" } }]}',
      ),
    ).toMatch(/unknown field "modle"/);
  });

  it("treats explicit null optional fields as absent (round-trip tolerance)", () => {
    // Exactly the shape an older canonical serialize produced — must validate.
    const raw =
      '{"enabled":true,"rules":[{"when":{"difficulty":null,"kind":null,"titleContains":"css"},"target":{"agent":"cursor","model":null}}],"fallback":{"agent":"claude","model":"sonnet"}}';
    expect(validateRoutingPolicy(raw)).toBeNull();
  });

  it("rejects a blank titleContains (inert clause, not a wildcard)", () => {
    expect(
      validateRoutingPolicy(
        '{"enabled": true, "rules": [{ "when": { "titleContains": "  " }, "target": { "agent": "cursor" } }]}',
      ),
    ).toMatch(/titleContains must not be blank/);
  });
});
