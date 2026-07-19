#!/usr/bin/env node

// workspace.report.js — dependency complexity report for the multi-workspace tree.
//
// For every internal crate, answers: "if this crate changes, how many other crates
// recompile?" (transitive reverse-dependency count, the blast radius).
//
// Read-only over the repo: reads Cargo.toml files, writes only the report file.
//
//   node workspace.report.js                 # write workspace.report.html at repo root
//   node workspace.report.js --out FILE      # write the HTML elsewhere
//   node workspace.report.js --json          # metrics as JSON on stdout (no file written)
//   node workspace.report.js --crate NAME    # plain-text detail for one crate

const fs = require('fs');
const path = require('path');

const REPO_ROOT = __dirname;

// Same standalone-tree exclusions as workspace.link.js: these roots are never
// consumed cross-workspace, so they are out of the recompile graph.
const EXCLUDE = [/compositor\.installer/, /developer\.tool/, /[\/\\]vendor[\/\\]/, /[\/\\]target[\/\\]/];

/** @returns {string[]} absolute workspace-root directories (see workspace.link.js) */
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

/**
 * @param {string} absWorkspaceDir
 * @returns {Map<string, string>} crate name -> absolute crate directory
 */
function getWorkspaceCratesManually(absWorkspaceDir) {
  const rootTomlPath = path.join(absWorkspaceDir, 'Cargo.toml');
  const crates = new Map();
  if (!fs.existsSync(rootTomlPath)) return crates;

  const rootToml = fs.readFileSync(rootTomlPath, 'utf-8');
  const membersMatch = rootToml.match(/members\s*=\s*\[([\s\S]*?)\]/);
  if (!membersMatch) return crates;

  const arrayBody = membersMatch[1].replace(/#[^\n]*/g, '');
  const rawMembers = Array.from(arrayBody.matchAll(/["']([^"']+)["']/g))
    .map(m => m[1])
    .filter(s => s.length > 0);

  const memberDirs = new Set();
  for (const member of rawMembers) {
    if (/[*?[\]]/.test(member)) {
      const pattern = path.posix.join(member.split(path.sep).join('/'), 'Cargo.toml');
      let matches;
      try { matches = fs.globSync(pattern, { cwd: absWorkspaceDir }); } catch (_) { continue; }
      for (const rel of matches) memberDirs.add(path.dirname(path.resolve(absWorkspaceDir, rel)));
    } else {
      memberDirs.add(path.resolve(absWorkspaceDir, member));
    }
  }

  for (const memberDir of memberDirs) {
    const tomlPath = path.join(memberDir, 'Cargo.toml');
    if (!fs.existsSync(tomlPath)) continue;
    const tomlContent = fs.readFileSync(tomlPath, 'utf-8');
    const packageBlockMatch = tomlContent.match(/\[package\]([\s\S]*?)(?:\n\[|$)/);
    if (!packageBlockMatch) continue;
    const nameMatch = packageBlockMatch[1].match(/name\s*=\s*["']([^"']+)["']/);
    if (nameMatch) crates.set(nameMatch[1], memberDir);
  }
  return crates;
}

/**
 * Dependency names declared in a crate manifest, across [dependencies],
 * [dev-dependencies], [build-dependencies] and target-specific variants.
 * Covers both `foo.workspace = true` and `foo = { workspace = true }`.
 * @param {string} crateDir
 * @returns {Set<string>}
 */
function parseDeclaredDeps(crateDir) {
  const toml = fs.readFileSync(path.join(crateDir, 'Cargo.toml'), 'utf-8');
  const deps = new Set();
  let inDepTable = false;
  for (const rawLine of toml.split('\n')) {
    const line = rawLine.replace(/#.*$/, '').trim();
    if (!line) continue;
    const header = line.match(/^\[(.+)\]$/);
    if (header) {
      inDepTable = /(^|\.)(dependencies|dev-dependencies|build-dependencies)$/.test(header[1]);
      continue;
    }
    if (!inDepTable) continue;
    const key = line.match(/^([A-Za-z0-9_-]+)\s*[=.]/);
    if (key) deps.add(key[1]);
  }
  return deps;
}

/** BFS reachability size over an adjacency map (excluding the start node). */
function reachable(adj, start) {
  const seen = new Set([start]);
  const queue = [start];
  while (queue.length) {
    for (const next of adj.get(queue.shift()) || []) {
      if (!seen.has(next)) { seen.add(next); queue.push(next); }
    }
  }
  seen.delete(start);
  return seen;
}

function buildModel() {
  const roots = discoverWorkspaceRoots();
  /** @type {Map<string, {dir: string, root: string}>} */
  const crates = new Map();
  for (const root of roots) {
    for (const [name, dir] of getWorkspaceCratesManually(root)) {
      crates.set(name, { dir, root: path.relative(REPO_ROOT, root) });
    }
  }

  const forward = new Map();   // crate -> internal deps
  const reverse = new Map();   // crate -> direct dependents
  for (const name of crates.keys()) { forward.set(name, new Set()); reverse.set(name, new Set()); }
  for (const [name, { dir }] of crates) {
    for (const dep of parseDeclaredDeps(dir)) {
      if (dep !== name && crates.has(dep)) {
        forward.get(name).add(dep);
        reverse.get(dep).add(name);
      }
    }
  }

  const rows = [];
  for (const [name, { dir, root }] of crates) {
    const dependents = reachable(reverse, name);
    rows.push({
      name,
      workspace: root,
      path: path.relative(REPO_ROOT, dir),
      directDeps: forward.get(name).size,
      transitiveDeps: reachable(forward, name).size,
      directDependents: reverse.get(name).size,
      transitiveDependents: dependents.size,
    });
  }
  rows.sort((a, b) => b.transitiveDependents - a.transitiveDependents || a.name.localeCompare(b.name));
  return { roots, crates, forward, reverse, rows };
}

function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function renderHtml({ roots, rows }) {
  const total = rows.length;
  const others = Math.max(1, total - 1);
  const maxBlast = rows.reduce((m, r) => Math.max(m, r.transitiveDependents), 0);
  const avgBlast = total ? rows.reduce((s, r) => s + r.transitiveDependents, 0) / total : 0;

  const perWorkspace = new Map();
  for (const r of rows) {
    const w = perWorkspace.get(r.workspace) || { crates: 0, max: 0, sum: 0 };
    w.crates += 1;
    w.max = Math.max(w.max, r.transitiveDependents);
    w.sum += r.transitiveDependents;
    perWorkspace.set(r.workspace, w);
  }
  const wsRows = Array.from(perWorkspace.entries())
    .sort((a, b) => b[1].max - a[1].max || a[0].localeCompare(b[0]))
    .map(([ws, w]) => `<tr><td class="name">${escapeHtml(ws)}</td>` +
      `<td class="num">${w.crates}</td><td class="num">${w.max}</td>` +
      `<td class="num">${(w.sum / w.crates).toFixed(1)}</td></tr>`)
    .join('\n');

  const crateRows = rows.map((r, i) => {
    const pct = (r.transitiveDependents / others) * 100;
    return `<tr title="${escapeHtml(r.path)}">` +
      `<td class="num rank">${i + 1}</td>` +
      `<td class="name">${escapeHtml(r.name)}<span class="ws">${escapeHtml(r.workspace)}</span></td>` +
      `<td class="bar-cell"><span class="bar" style="width:${pct.toFixed(2)}%"></span>` +
      `<span class="bar-label">${r.transitiveDependents}</span></td>` +
      `<td class="num">${pct.toFixed(1)}%</td>` +
      `<td class="num">${r.directDependents}</td>` +
      `<td class="num">${r.directDeps}</td>` +
      `<td class="num">${r.transitiveDeps}</td></tr>`;
  }).join('\n');

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>y5 dependency complexity report</title>
<style>
  :root {
    color-scheme: light;
    --surface-1: #fcfcfb;
    --surface-2: #f1f1ee;
    --border: #dddcd6;
    --text-primary: #0b0b0b;
    --text-secondary: #52514e;
    --series-1: #2a78d6;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      color-scheme: dark;
      --surface-1: #1a1a19;
      --surface-2: #242423;
      --border: #3a3a38;
      --text-primary: #ffffff;
      --text-secondary: #c3c2b7;
      --series-1: #3987e5;
    }
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 2rem 1.5rem;
    background: var(--surface-1); color: var(--text-primary);
    font: 14px/1.5 system-ui, sans-serif;
  }
  main { max-width: 72rem; margin: 0 auto; }
  h1 { font-size: 1.3rem; margin: 0 0 .25rem; }
  h2 { font-size: 1rem; margin: 2rem 0 .5rem; }
  .subtitle { color: var(--text-secondary); margin: 0 0 1.5rem; }
  .tiles { display: flex; flex-wrap: wrap; gap: .75rem; margin: 1rem 0 1.5rem; }
  .tile {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 8px; padding: .6rem 1rem; min-width: 9rem;
  }
  .tile .v { font-size: 1.4rem; font-weight: 600; }
  .tile .k { color: var(--text-secondary); font-size: .8rem; }
  .table-wrap { overflow-x: auto; border: 1px solid var(--border); border-radius: 8px; }
  table { border-collapse: collapse; width: 100%; }
  th, td { padding: .35rem .6rem; text-align: left; white-space: nowrap; }
  thead th {
    position: sticky; top: 0; background: var(--surface-2);
    color: var(--text-secondary); font-weight: 600; font-size: .78rem;
    border-bottom: 1px solid var(--border);
    white-space: normal; vertical-align: bottom;
  }
  tbody tr:hover { background: var(--surface-2); }
  tbody tr + tr td { border-top: 1px solid var(--border); }
  td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
  td.rank { color: var(--text-secondary); }
  td.name { font-family: ui-monospace, monospace; font-size: .82rem; }
  td.name .ws { display: block; color: var(--text-secondary); font-size: .72rem; }
  td.bar-cell { min-width: 12rem; width: 30%; position: relative; }
  .bar {
    display: inline-block; height: 12px; min-width: 1px;
    background: var(--series-1); border-radius: 0 4px 4px 0;
    vertical-align: middle;
  }
  .bar-label {
    margin-left: .5rem; font-variant-numeric: tabular-nums;
    color: var(--text-primary); vertical-align: middle;
  }
  footer { color: var(--text-secondary); font-size: .78rem; margin-top: 1.5rem; }
</style>
</head>
<body>
<main>
  <h1>Dependency complexity report</h1>
  <p class="subtitle">Blast radius per crate: how many other crates recompile when it changes.
  Sorted most-blasting first.</p>

  <div class="tiles">
    <div class="tile"><div class="v">${total}</div><div class="k">internal crates</div></div>
    <div class="tile"><div class="v">${roots.length}</div><div class="k">workspace roots</div></div>
    <div class="tile"><div class="v">${maxBlast}</div><div class="k">max blast radius</div></div>
    <div class="tile"><div class="v">${avgBlast.toFixed(1)}</div><div class="k">avg blast radius</div></div>
  </div>

  <div class="table-wrap">
  <table>
    <thead><tr>
      <th class="num">#</th><th>crate</th>
      <th>transitive dependents (blast radius)</th><th class="num">% of tree</th>
      <th class="num">direct dependents</th>
      <th class="num">direct deps</th><th class="num">transitive deps</th>
    </tr></thead>
    <tbody>
${crateRows}
    </tbody>
  </table>
  </div>

  <h2>Per workspace root</h2>
  <div class="table-wrap">
  <table>
    <thead><tr>
      <th>workspace</th><th class="num">crates</th>
      <th class="num">max blast radius</th><th class="num">avg blast radius</th>
    </tr></thead>
    <tbody>
${wsRows}
    </tbody>
  </table>
  </div>

  <footer>Generated by workspace.report.js — internal crates only (vendor/, installer and
  developer.tool trees excluded). Hover a row for the crate path.</footer>
</main>
</body>
</html>
`;
}

function main() {
  const argv = process.argv.slice(2);
  const flag = (name) => {
    const i = argv.indexOf(name);
    return i === -1 ? undefined : (argv[i + 1] ?? true);
  };

  const model = buildModel();

  if (argv.includes('--json')) {
    process.stdout.write(JSON.stringify({
      generatedFor: path.basename(REPO_ROOT),
      crateCount: model.rows.length,
      workspaceRoots: model.roots.map(r => path.relative(REPO_ROOT, r)),
      crates: model.rows,
    }, null, 2) + '\n');
    return;
  }

  const crateName = flag('--crate');
  if (typeof crateName === 'string') {
    if (!model.crates.has(crateName)) {
      console.error(`[Error] Unknown crate '${crateName}'. ${model.crates.size} internal crates known.`);
      process.exit(1);
    }
    const dependents = reachable(model.reverse, crateName);
    const direct = model.reverse.get(crateName);
    console.log(`${crateName}`);
    console.log(`  path: ${path.relative(REPO_ROOT, model.crates.get(crateName).dir)}`);
    console.log(`  blast radius: ${dependents.size} crate(s) recompile on change\n`);
    console.log(`  direct dependents (${direct.size}):`);
    for (const d of Array.from(direct).sort()) console.log(`    ${d}`);
    const indirect = Array.from(dependents).filter(d => !direct.has(d)).sort();
    console.log(`  transitive-only dependents (${indirect.length}):`);
    for (const d of indirect) console.log(`    ${d}`);
    return;
  }

  const out = typeof flag('--out') === 'string'
    ? path.resolve(flag('--out'))
    : path.join(REPO_ROOT, 'workspace.report.html');
  fs.writeFileSync(out, renderHtml(model), 'utf-8');
  console.log(`Analyzed ${model.rows.length} crates across ${model.roots.length} workspace roots.`);
  console.log(`Report written to: ${out}`);
}

main();
