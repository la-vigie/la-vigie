#!/usr/bin/env node
// TASK-35: generate per-provider La Vigie skill bundles from the single
// canonical source in .rulesync/skills/, and vendor them as Tauri resources.
//
// rulesync writes each tool's skills into its native cwd-relative layout
// (.claude/skills, .agents/skills, .opencode/skills, .vibe/skills). We run it in
// a per-target staging dir (a copy of .rulesync), then copy the produced dotdirs
// into the committed resource tree. Running in a staging cwd keeps us agnostic to
// rulesync's base-dir flags — it always writes relative to cwd.
import { execFileSync } from 'node:child_process';
import { cpSync, mkdtempSync, mkdirSync, rmSync, readdirSync, existsSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '..');
const SRC = join(ROOT, '.rulesync');
const SKILLS_OUT = join(ROOT, 'src-tauri/resources/lavigie-skills');
const MCP_OUT = join(ROOT, 'src-tauri/resources/lavigie-mcp');
const PLUGIN_SKILLS = join(ROOT, 'src-tauri/resources/lavigie-plugin/skills');

// Invoke the repo's PINNED rulesync (package.json exact-pins it) directly by its
// entry file, run under the current `node`. NOT `npx rulesync`: the staging cwd
// below is a tmpdir outside the repo, so `npx` can't resolve the local install
// and would silently download a different (wrong) version — the exact
// nondeterminism this vendored pipeline exists to avoid. Requires `npm ci` first
// (and Node satisfying rulesync's `engines`, currently >=22).
const RULESYNC = join(ROOT, 'node_modules', 'rulesync', 'dist', 'cli', 'index.js');

// rulesync target → vendored provider dir (matches AgentSpec provider keys).
const PROVIDERS = [
  { target: 'codexcli', dir: 'codex' },
  { target: 'antigravity-cli', dir: 'antigravity' },
  { target: 'opencode', dir: 'opencode' },
  { target: 'vibe', dir: 'mistral' },
];

function generateFeature(feature, target, cwd) {
  if (!existsSync(RULESYNC)) {
    throw new Error(`pinned rulesync not found at ${RULESYNC} — run \`npm ci\` first`);
  }
  execFileSync(process.execPath, [RULESYNC, 'generate', '--features', feature, '--targets', target], {
    cwd, stdio: 'inherit',
  });
}

// Copy every produced entry except the source `.rulesync` dir into dest.
function vendor(stagingDir, destDir) {
  rmSync(destDir, { recursive: true, force: true });
  for (const name of readdirSync(stagingDir)) {
    if (name === '.rulesync') continue;
    cpSync(join(stagingDir, name), join(destDir, name), { recursive: true });
  }
}

function run() {
  // Wipe the per-provider output roots once up front so a provider removed from
  // PROVIDERS can't leave a stale vendored dir behind (which would defeat the
  // drift guard). The per-provider vendor() cleanup below is kept too.
  rmSync(SKILLS_OUT, { recursive: true, force: true });
  rmSync(MCP_OUT, { recursive: true, force: true });

  // Per-provider skill bundles.
  for (const { target, dir } of PROVIDERS) {
    const staging = mkdtempSync(join(tmpdir(), `rulesync-${dir}-`));
    try {
      cpSync(SRC, join(staging, '.rulesync'), { recursive: true });
      generateFeature('skills', target, staging);
      vendor(staging, join(SKILLS_OUT, dir));
    } finally {
      rmSync(staging, { recursive: true, force: true });
    }
  }

  // Per-provider MCP config (TASK-193). A SEPARATE rulesync pass (--features mcp,
  // reading .rulesync/.mcp.json) vendored into a SEPARATE tree, so the skill
  // bundles above stay byte-identical. Claude is intentionally excluded — it gets
  // an inline `--mcp-config` at spawn, not a vendored file. The per-spawn port +
  // bearer token stay as `__LAVIGIE_MCP_PORT__`/`__LAVIGIE_MCP_TOKEN__` sentinels
  // here; agent/mcp_bundle.rs substitutes them when materializing at launch.
  for (const { target, dir } of PROVIDERS) {
    const staging = mkdtempSync(join(tmpdir(), `rulesync-mcp-${dir}-`));
    try {
      cpSync(SRC, join(staging, '.rulesync'), { recursive: true });
      generateFeature('mcp', target, staging);
      vendor(staging, join(MCP_OUT, dir));
    } finally {
      rmSync(staging, { recursive: true, force: true });
    }
  }

  // Codex fixup (TASK-193): rulesync emits a `[mcp_servers.lavigie.headers]`
  // Authorization table, but Codex has NO static-header auth for HTTP MCP — it
  // reads the bearer token from the env var named by `bearer_token_env_var` (see
  // `codex mcp add --bearer-token-env-var`). Overwrite Codex's generated config
  // with that native form; La Vigie sets LAVIGIE_MCP_TOKEN in the agent's spawn
  // env, so Codex's token never touches disk. Only the port is a substituted
  // placeholder here. (The other three engines honor the static header, so their
  // rulesync output is left as-is.)
  const codexCfg = join(MCP_OUT, 'codex', '.codex', 'config.toml');
  mkdirSync(join(MCP_OUT, 'codex', '.codex'), { recursive: true });
  writeFileSync(codexCfg,
    '# La Vigie MCP server (TASK-193). Codex reads the bearer token from the env\n' +
    '# var named below — it has no static-header auth for HTTP MCP — so La Vigie\n' +
    '# sets LAVIGIE_MCP_TOKEN in the agent\'s spawn environment. Generated by\n' +
    '# scripts/generate-skills.mjs; do not edit by hand.\n' +
    '[mcp_servers.lavigie]\n' +
    'url = "http://127.0.0.1:__LAVIGIE_MCP_PORT__/mcp"\n' +
    'bearer_token_env_var = "LAVIGIE_MCP_TOKEN"\n');

  // Claude plugin: generate claudecode skills (.claude/skills/*), then place them
  // under the plugin's skills/ dir. The static .claude-plugin/plugin.json stays
  // committed and untouched.
  const staging = mkdtempSync(join(tmpdir(), 'rulesync-claude-'));
  try {
    cpSync(SRC, join(staging, '.rulesync'), { recursive: true });
    generateFeature('skills', 'claudecode', staging);
    const produced = join(staging, '.claude', 'skills');
    if (!existsSync(produced)) throw new Error('rulesync produced no .claude/skills');
    rmSync(PLUGIN_SKILLS, { recursive: true, force: true });
    cpSync(produced, PLUGIN_SKILLS, { recursive: true });
    // Safety net: ensure sibling asset files (e.g. await-merge's helper script)
    // that reference ${CLAUDE_PLUGIN_ROOT} are present in the plugin bundle.
    for (const name of readdirSync(join(SRC, 'skills'))) {
      const srcDir = join(SRC, 'skills', name);
      mkdirSync(join(PLUGIN_SKILLS, name), { recursive: true });
      for (const f of readdirSync(srcDir)) {
        if (f === 'SKILL.md') continue;
        cpSync(join(srcDir, f), join(PLUGIN_SKILLS, name, f));
      }
    }
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
  console.log('Generated La Vigie skill bundles.');
}

run();
