#!/usr/bin/env node

const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

// --- CONFIGURATION ---
// Define your target workspace and the array of input workspaces here.
// These can be absolute paths or relative to where you execute the script.

const TARGET_WORKSPACE = process.cwd();

// Repo root = the directory this script lives in (link.all.sh invokes it as
// `node ../workspace.link.js` / `../../workspace.link.js` from each root).
const REPO_ROOT = __dirname;

// Workspace roots NOT part of the global internal-crate pool. These are standalone
// trees (their crates are never consumed cross-workspace via `workspace = true`):
//  - compositor.installer.*  — standalone installer (see installer-standalone memory)
//  - developer.tool.* — dev tools, out of the link graph (nested under
//    compositor.developer/developer.tool, the developer container).
// `vendor/` and `target/` are never workspace roots we generate into.
const EXCLUDE = [/compositor\.installer/, /developer\.tool/, /[\/\\]vendor[\/\\]/, /[\/\\]target[\/\\]/];

/**
 * Discover every linked workspace root in the repo (Cargo.toml with a [workspace]
 * members array), so any internal crate is reachable from any root via
 * `{name}.workspace = true` — generation is GLOBAL, not per-root link.json.
 * Roots live at depth 1 (compositor.orchestration) or depth 2
 * (compositor.support/support.smithay, compositor.kernel/kernel.x, ...).
 * @returns {string[]} absolute workspace-root directories
 */
function discoverWorkspaceRoots() {
  const patterns = ['compositor.*/Cargo.toml', 'compositor.*/*/Cargo.toml'];
  const roots = new Set();
  for (const pattern of patterns) {
    let matches = [];
    try { matches = fs.globSync(pattern, { cwd: REPO_ROOT }); } catch (_) { continue; }
    for (const rel of matches) {
      if (EXCLUDE.some(re => re.test(rel))) continue;
      const dir = path.dirname(path.resolve(REPO_ROOT, rel));
      const toml = fs.readFileSync(path.join(dir, 'Cargo.toml'), 'utf-8');
      if (/\[workspace\]/.test(toml) && /members\s*=\s*\[/.test(toml)) roots.add(dir);
    }
  }
  return Array.from(roots);
}

const INPUT_WORKSPACES = discoverWorkspaceRoots();

// Optional per-root feature overrides: a `link.features.json` in the target root maps
// a generated crate name to feature attributes the root controls (feature selection
// lives at root, never in a crate). Example:
//   { "compositor_model_debug_instance_record":
//       { "default-features": false, "features": ["error","warn","info","trace"] } }
function loadFeatureOverrides() {
  const p = path.join(TARGET_WORKSPACE, 'link.features.json');
  if (!fs.existsSync(p)) return {};
  try { return JSON.parse(fs.readFileSync(p, 'utf-8')); }
  catch (e) { console.warn(`[Warning] failed to parse link.features.json: ${e.message}`); return {}; }
}

// Discovery mode: "cargo" uses `cargo metadata`, "manual" parses Cargo.toml directly.
// Manual mode is useful when a workspace fails cargo's strict validation
// (e.g. missing crates, broken members) but you still want to link what's there.
// const DISCOVERY_MODE = process.env.LINK_MODE || 'cargo'; // 'cargo' | 'manual'
const DISCOVERY_MODE = "manual";
// ---------------------

/**
 * Runs `cargo metadata` in a given workspace and returns a map of its local crates.
 * @param {string} workspaceDir
 * @returns {Map<string, string>} Map of crate name to its absolute directory path
 */
function getWorkspaceCratesViaCargo(workspaceDir) {
  const absWorkspaceDir = path.resolve(workspaceDir);
  console.log(`Scanning input workspace via cargo: ${absWorkspaceDir}`);

  let metadataStr;
  try {
    // --no-deps ensures we only get information about the workspace itself, fast.
    metadataStr = execSync('cargo metadata --format-version 1 --no-deps', {
      cwd: absWorkspaceDir,
      encoding: 'utf-8',
      stdio: ['pipe', 'pipe', 'ignore'] // Suppress stderr noise
    });
  } catch (e) {
    console.error(`[Error] Failed to run cargo metadata in ${absWorkspaceDir}. Ensure it is a valid Rust workspace.`);
    process.exit(1);
  }

  const metadata = JSON.parse(metadataStr);
  const members = new Set(metadata.workspace_members);
  const crates = new Map();

  metadata.packages.forEach(pkg => {
    // Only extract crates that are actual members of this workspace
    if (members.has(pkg.id)) {
      // pkg.manifest_path is the absolute path to Cargo.toml
      // We need the directory containing it for the `path` directive
      const crateDir = path.dirname(pkg.manifest_path);
      crates.set(pkg.name, crateDir);
    }
  });

  return crates;
}

/**
 * Manually parses Cargo.toml to find local crates, bypassing cargo's strict validation.
 * Uses Node's built-in fs.globSync so glob patterns like "crates/*" resolve correctly,
 * including more complex patterns like "crates/**\/*" or "libs/*-core".
 * @param {string} workspaceDir
 * @returns {Map<string, string>} Map of crate name to its absolute directory path
 */
function getWorkspaceCratesManually(workspaceDir) {
  const absWorkspaceDir = path.resolve(workspaceDir);
  console.log(`Scanning input workspace manually: ${absWorkspaceDir}`);

  const rootTomlPath = path.join(absWorkspaceDir, 'Cargo.toml');
  const crates = new Map();

  if (!fs.existsSync(rootTomlPath)) {
    console.warn(`[Warning] No Cargo.toml found at ${rootTomlPath}`);
    return crates;
  }

  const rootToml = fs.readFileSync(rootTomlPath, 'utf-8');

  // 1. Extract the workspace members array.
  // Matches: members = [ "crate1", "crates/*" ]
  // We accept the array appearing under [workspace] or as a bare `members = [...]`.
  const membersMatch = rootToml.match(/members\s*=\s*\[([\s\S]*?)\]/);
  if (!membersMatch) {
    console.warn(`[Warning] No [workspace] members array found in ${rootTomlPath}`);
    return crates;
  }

  // Strip line comments, then pull out quoted entries.
  const arrayBody = membersMatch[1].replace(/#[^\n]*/g, '');
  const rawMembers = Array.from(arrayBody.matchAll(/["']([^"']+)["']/g))
    .map(m => m[1])
    .filter(s => s.length > 0);

  // 2. Resolve each member entry (plain path or glob) to concrete directories
  // via fs.globSync, which understands "*", "**", "?", "[...]", etc.
  const memberDirs = new Set();
  for (const member of rawMembers) {
    const hasGlob = /[*?[\]]/.test(member);

    if (hasGlob) {
      // globSync with `cwd` returns paths relative to cwd; we want absolute dirs.
      // We append /Cargo.toml to the pattern so we only match actual crate dirs,
      // then strip it back off. This avoids matching arbitrary non-crate folders.
      const pattern = path.posix.join(
        member.split(path.sep).join('/'),
        'Cargo.toml'
      );

      let matches;
      try {
        matches = fs.globSync(pattern, { cwd: absWorkspaceDir });
      } catch (e) {
        console.warn(`[Warning] globSync failed for pattern '${pattern}': ${e.message}`);
        continue;
      }

      for (const rel of matches) {
        memberDirs.add(path.dirname(path.resolve(absWorkspaceDir, rel)));
      }
    } else {
      memberDirs.add(path.resolve(absWorkspaceDir, member));
    }
  }

  // 3. Extract the package name from each member's Cargo.toml
  for (const memberDir of memberDirs) {
    const tomlPath = path.join(memberDir, 'Cargo.toml');
    if (!fs.existsSync(tomlPath)) continue;

    const tomlContent = fs.readFileSync(tomlPath, 'utf-8');

    // Find the [package] block and pull `name = "..."` from inside it.
    const packageBlockMatch = tomlContent.match(/\[package\]([\s\S]*?)(?:\n\[|$)/);
    if (!packageBlockMatch) continue;

    const nameMatch = packageBlockMatch[1].match(/name\s*=\s*["']([^"']+)["']/);
    if (nameMatch) {
      crates.set(nameMatch[1], memberDir);
    }
  }

  return crates;
}

/**
 * Dispatch to the appropriate discovery strategy.
 * @param {string} workspaceDir
 * @param {string} mode 'cargo' | 'manual'
 */
function getWorkspaceCrates(workspaceDir, mode = DISCOVERY_MODE) {
  switch (mode) {
    case 'cargo':
      return getWorkspaceCratesViaCargo(workspaceDir);
    case 'manual':
      return getWorkspaceCratesManually(workspaceDir);
    default:
      console.error(`[Error] Unknown discovery mode: '${mode}'. Use 'cargo' or 'manual'.`);
      process.exit(1);
  }
}

function main() {
  const targetAbsPath = path.resolve(TARGET_WORKSPACE);
  const targetTomlPath = path.join(targetAbsPath, 'Cargo.toml');

  if (!fs.existsSync(targetTomlPath)) {
    console.error(`[Error] Target Cargo.toml not found at: ${targetTomlPath}`);
    process.exit(1);
  }

  console.log(`Discovery mode: ${DISCOVERY_MODE}`);

  // 1. Harvest all local crates from the input workspaces
  const allLocalCrates = new Map();

  for (const ws of INPUT_WORKSPACES) {
    const crates = getWorkspaceCrates(ws, DISCOVERY_MODE);
    for (const [name, cratePath] of crates.entries()) {
      if (allLocalCrates.has(name)) {
        console.warn(`[Warning] Crate collision detected for '${name}'. Overwriting with path from ${ws}`);
      }
      allLocalCrates.set(name, cratePath);
    }
  }

  if (allLocalCrates.size === 0) {
    console.log("No workspace members found in the input workspaces. Exiting.");
    return;
  }

  console.log(`\nFound ${allLocalCrates.size} total crates. Linking to target workspace...`);

  // 2. Compute relative paths and generate the TOML block
  // Alphabetical sort ensures deterministic, clean diffs upon re-running
  const sortedCrates = Array.from(allLocalCrates.entries()).sort((a, b) => a[0].localeCompare(b[0]));

  const featureOverrides = loadFeatureOverrides();

  let tomlSection = '\n\n# --- GENERATED WORKSPACE LINKS START ---\n';
  tomlSection += '\n';

  for (const [name, absCrateDir] of sortedCrates) {
    // Calculate the relative path strictly from the target workspace root to the crate directory
    let relPath = path.relative(targetAbsPath, absCrateDir);

    // Normalize path separators just in case (Cargo prefers forward slashes)
    relPath = relPath.split(path.sep).join('/');

    if (relPath === '') relPath = '.';

    // Root-controlled feature selection (link.features.json), emitted inline so the
    // root owns features even for a generated cross-workspace dep.
    const ov = featureOverrides[name];
    let attrs = `path = "${relPath}"`;
    if (ov) {
      if (ov['default-features'] === false) attrs += `, default-features = false`;
      if (Array.isArray(ov.features)) attrs += `, features = [${ov.features.map(f => `"${f}"`).join(', ')}]`;
    }
    tomlSection += `${name} = { ${attrs} }\n`;
  }

  tomlSection += '# --- GENERATED WORKSPACE LINKS END ---';

  // 3. Inject or update the target Cargo.toml. The block MUST live inside the
  // [workspace.dependencies] table — appending at EOF breaks if a later table
  // (e.g. [patch.crates-io]) follows, since the generated keys would fall under
  // that table instead. So: strip any existing block, then insert at the END of
  // the [workspace.dependencies] section (right before the next table header, or
  // EOF if it's the last table).
  let tomlContent = fs.readFileSync(targetTomlPath, 'utf-8');
  const sectionRegex = /\n*# --- GENERATED WORKSPACE LINKS START ---[\s\S]*?# --- GENERATED WORKSPACE LINKS END ---/g;

  // Remove the previous generated block wherever it sits.
  tomlContent = tomlContent.replace(sectionRegex, '').trimEnd();

  const wsDepsMatch = tomlContent.match(/^\[workspace\.dependencies\][ \t]*$/m);
  if (wsDepsMatch) {
    const wsDepsStart = wsDepsMatch.index + wsDepsMatch[0].length;
    // Next table header after [workspace.dependencies], if any.
    const after = tomlContent.slice(wsDepsStart);
    const nextHeader = after.search(/^\[/m);
    if (nextHeader === -1) {
      // [workspace.dependencies] is the last table — append the block at EOF.
      tomlContent = tomlContent + tomlSection + '\n';
    } else {
      const insertAt = wsDepsStart + nextHeader;
      tomlContent = tomlContent.slice(0, insertAt).trimEnd() + tomlSection + '\n\n' + tomlContent.slice(insertAt);
    }
    console.log("-> Synced generated links inside [workspace.dependencies].");
  } else {
    // No [workspace.dependencies] table — create one with the block.
    tomlContent = tomlContent + '\n\n[workspace.dependencies]' + tomlSection + '\n';
    console.log("-> Created [workspace.dependencies] with generated links.");
  }

  fs.writeFileSync(targetTomlPath, tomlContent, 'utf-8');
  console.log("Done! Target Cargo.toml is synchronized.");
}

main();