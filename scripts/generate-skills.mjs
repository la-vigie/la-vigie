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
import { cpSync, mkdtempSync, mkdirSync, rmSync, readdirSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '..');
const SRC = join(ROOT, '.rulesync');
const SKILLS_OUT = join(ROOT, 'src-tauri/resources/lavigie-skills');
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

function generate(target, cwd) {
  if (!existsSync(RULESYNC)) {
    throw new Error(`pinned rulesync not found at ${RULESYNC} — run \`npm ci\` first`);
  }
  execFileSync(process.execPath, [RULESYNC, 'generate', '--features', 'skills', '--targets', target], {
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
  // Wipe the per-provider output root once up front so a provider removed from
  // PROVIDERS can't leave a stale vendored dir behind (which would defeat the
  // Task 6 drift guard). The per-provider vendor() cleanup below is kept too.
  rmSync(SKILLS_OUT, { recursive: true, force: true });

  // Per-provider bundles.
  for (const { target, dir } of PROVIDERS) {
    const staging = mkdtempSync(join(tmpdir(), `rulesync-${dir}-`));
    try {
      cpSync(SRC, join(staging, '.rulesync'), { recursive: true });
      generate(target, staging);
      vendor(staging, join(SKILLS_OUT, dir));
    } finally {
      rmSync(staging, { recursive: true, force: true });
    }
  }

  // Claude plugin: generate claudecode skills (.claude/skills/*), then place them
  // under the plugin's skills/ dir. The static .claude-plugin/plugin.json stays
  // committed and untouched.
  const staging = mkdtempSync(join(tmpdir(), 'rulesync-claude-'));
  try {
    cpSync(SRC, join(staging, '.rulesync'), { recursive: true });
    generate('claudecode', staging);
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
