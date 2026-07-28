#!/usr/bin/env node

// workspace.report.js — dependency complexity + build report for the multi-workspace tree.
//
// For every internal crate, answers: "if this crate changes, how many other crates
// recompile?" (transitive reverse-dependency count, the blast radius).
//
// When release build artifacts exist (produced by `environment/build.sh udev release`),
// the report also includes a build section: binary size broken down by ELF section and
// by crate (nm symbol attribution), per-crate compile times (cargo --timings data), the
// largest .rlib artifacts, and the compiler flags in effect. The section is skipped —
// with a hint — when no build/timings are present. To refresh the underlying data:
//
//   environment/build.sh udev release        # (add --timings via: cargo build --timings
//                                            #  in the loader workspace, same features)
//
// Read-only over the repo: reads Cargo.toml files and build artifacts, writes only the
// report file.
//
//   node workspace.report.js                 # write workspace.report.html at repo root
//   node workspace.report.js --out FILE      # write the HTML elsewhere
//   node workspace.report.js --json          # metrics as JSON on stdout (no file written)
//   node workspace.report.js --crate NAME    # plain-text detail for one crate

const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

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

// --------------------------------------------------------------------------
// Build report (binary size + compile time), sourced from the release target
// dir of the workspace that owns the y5_compositor [[bin]]. Everything here is
// best-effort: any missing artifact/tool just drops its table from the report.
// --------------------------------------------------------------------------

/**
 * The crate declaring the y5_compositor [[bin]] and its workspace's target dir
 * (same discovery rule as environment/build.sh; Y5_TARGET_DIR wins if set).
 * @param {Map<string, {dir: string}>} crates
 * @returns {{crateDir: string, targetDir: string} | null}
 */
function findBuildTarget(crates) {
  for (const { dir } of crates.values()) {
    const toml = fs.readFileSync(path.join(dir, 'Cargo.toml'), 'utf-8');
    if (!/\[\[bin\]\][\s\S]*?name\s*=\s*["']y5_compositor["']/.test(toml)) continue;
    if (process.env.Y5_TARGET_DIR) return { crateDir: dir, targetDir: process.env.Y5_TARGET_DIR };
    let ws = dir;
    while (ws !== path.dirname(ws)) {
      ws = path.dirname(ws);
      const t = path.join(ws, 'Cargo.toml');
      if (fs.existsSync(t) && /\[workspace\]/.test(fs.readFileSync(t, 'utf-8'))) {
        return { crateDir: dir, targetDir: path.join(ws, 'target') };
      }
    }
    return null;
  }
  return null;
}

/**
 * Per-crate compile times from the newest cargo --timings HTML report
 * (cargo embeds the unit list as a `const UNIT_DATA = [...]` JSON literal).
 * @param {string} targetDir
 * @returns {{wallSeconds: number, cpuSeconds: number, unitCount: number,
 *            generatedAt: string, crates: {name: string, cpuSeconds: number, units: number}[]} | null}
 */
function parseTimings(targetDir) {
  const dir = path.join(targetDir, 'cargo-timings');
  let file = path.join(dir, 'cargo-timing.html'); // cargo's copy of the newest report
  if (!fs.existsSync(file)) {
    let newest = null;
    for (const f of (fs.existsSync(dir) ? fs.readdirSync(dir) : [])) {
      if (!/^cargo-timing-.*\.html$/.test(f)) continue;
      const p = path.join(dir, f);
      if (!newest || fs.statSync(p).mtimeMs > fs.statSync(newest).mtimeMs) newest = p;
    }
    if (!newest) return null;
    file = newest;
  }
  const m = fs.readFileSync(file, 'utf-8').match(/const UNIT_DATA = (\[[\s\S]*?\]);/);
  if (!m) return null;
  let units;
  try { units = JSON.parse(m[1]); } catch (_) { return null; }

  const perCrate = new Map();
  let wall = 0;
  for (const u of units) {
    wall = Math.max(wall, u.start + u.duration);
    const c = perCrate.get(u.name) || { name: u.name, cpuSeconds: 0, units: 0 };
    c.cpuSeconds += u.duration;
    c.units += 1;
    perCrate.set(u.name, c);
  }
  const crates = Array.from(perCrate.values()).sort((a, b) => b.cpuSeconds - a.cpuSeconds);
  return {
    wallSeconds: wall,
    cpuSeconds: crates.reduce((s, c) => s + c.cpuSeconds, 0),
    unitCount: units.length,
    generatedAt: fs.statSync(file).mtime.toISOString(),
    crates,
  };
}

/**
 * ELF64 section table of the linked binary (pure-node parser; little-endian only).
 * @param {string} binPath
 * @returns {{name: string, size: number}[] | null}
 */
function parseElfSections(binPath) {
  const b = fs.readFileSync(binPath);
  if (b.length < 64 || b.readUInt32LE(0) !== 0x464c457f || b[4] !== 2 || b[5] !== 1) return null;
  const shoff = Number(b.readBigUInt64LE(0x28));
  const shentsize = b.readUInt16LE(0x3a);
  const shnum = b.readUInt16LE(0x3c);
  const shstrndx = b.readUInt16LE(0x3e);
  if (!shoff || !shnum || shstrndx >= shnum) return null;
  const strOff = Number(b.readBigUInt64LE(shoff + shstrndx * shentsize + 0x18));
  const sections = [];
  for (let i = 0; i < shnum; i++) {
    const off = shoff + i * shentsize;
    const nameOff = strOff + b.readUInt32LE(off);
    const end = b.indexOf(0, nameOff);
    const name = b.toString('ascii', nameOff, end === -1 ? nameOff : end);
    const size = Number(b.readBigUInt64LE(off + 0x20));
    if (name) sections.push({ name, size });
  }
  return sections.sort((a, b2) => b2.size - a.size);
}

/**
 * Attribute the binary's code (.text) bytes to crates via `nm` demangled symbol
 * names — first path segment of each sized t/T/w/W symbol. Monomorphized
 * generics land on the crate that DEFINES them (core/alloc/hashbrown read big
 * because everything instantiates them). Returns null when nm is unavailable.
 * @param {string} binPath
 * @returns {{name: string, bytes: number}[] | null}
 */
function attributeTextSymbols(binPath) {
  const nm = spawnSync('nm', ['--print-size', '--radix=d', '--demangle', binPath],
    { encoding: 'utf-8', maxBuffer: 1 << 30 });
  if (nm.status !== 0 || !nm.stdout) return null;
  const byCrate = new Map();
  for (const line of nm.stdout.split('\n')) {
    const m = line.match(/^\d+ (\d+) ([tTwW]) (.+)$/);
    if (!m) continue;
    const sym = m[3].replace(/^_?</, '');
    const c = sym.match(/^(?:dyn |&mut |&|\*const |\*mut )*([A-Za-z_][A-Za-z0-9_]*)(?:::|<)/);
    const crate = c ? c[1] : '(C / unmangled)';
    byCrate.set(crate, (byCrate.get(crate) || 0) + Number(m[1]));
  }
  return Array.from(byCrate, ([name, bytes]) => ({ name, bytes }))
    .sort((a, b) => b.bytes - a.bytes);
}

/**
 * Largest compiled .rlib per crate under target/release/deps. An rlib's size is
 * a compile-artifact metric (object code + metadata), NOT its share of the final
 * binary — the linker drops unused code (e.g. ash is huge here, small linked).
 * @param {string} targetDir
 * @returns {{name: string, bytes: number}[]}
 */
function collectRlibSizes(targetDir) {
  const deps = path.join(targetDir, 'release', 'deps');
  const byCrate = new Map();
  for (const f of (fs.existsSync(deps) ? fs.readdirSync(deps) : [])) {
    const m = f.match(/^lib(.+)-[0-9a-f]{16}\.rlib$/);
    if (!m) continue;
    const size = fs.statSync(path.join(deps, f)).size;
    if (size > (byCrate.get(m[1]) || 0)) byCrate.set(m[1], size);
  }
  return Array.from(byCrate, ([name, bytes]) => ({ name, bytes }))
    .sort((a, b) => b.bytes - a.bytes);
}

/** Rustflags from the repo-root .cargo/config.toml (see its RUSTFLAGS warning). */
function readRustflags() {
  const p = path.join(REPO_ROOT, '.cargo', 'config.toml');
  if (!fs.existsSync(p)) return [];
  const m = fs.readFileSync(p, 'utf-8').replace(/#[^\n]*/g, '').match(/rustflags\s*=\s*\[([\s\S]*?)\]/);
  return m ? Array.from(m[1].matchAll(/["']([^"']+)["']/g), x => x[1]) : [];
}

/**
 * Everything the build section renders; null when there is no release binary.
 * @param {Map<string, {dir: string}>} crates
 */
function buildBuildReport(crates) {
  const target = findBuildTarget(crates);
  if (!target) return null;
  const binPath = path.join(target.targetDir, 'release', 'y5_compositor');
  if (!fs.existsSync(binPath)) return null;
  const binStat = fs.statSync(binPath);

  // [profile.release] overrides in the bin-owning workspace root, as "k = v" strings.
  const wsToml = fs.readFileSync(path.join(path.dirname(target.targetDir), 'Cargo.toml'), 'utf-8');
  const prof = wsToml.replace(/#[^\n]*/g, '').match(/\[profile\.release\]([\s\S]*?)(?=\n\[|$)/);
  const profileOverrides = prof
    ? prof[1].split('\n').map(l => l.trim()).filter(l => /^[\w-]+\s*=/.test(l))
    : [];

  return {
    binPath,
    binBytes: binStat.size,
    binMtime: binStat.mtime.toISOString(),
    sections: parseElfSections(binPath),
    textByCrate: attributeTextSymbols(binPath),
    rlibs: collectRlibSizes(target.targetDir),
    timings: parseTimings(target.targetDir),
    rustflags: readRustflags(),
    profileOverrides,
  };
}

function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** 12345678 -> "11.8 MB" */
function fmtMB(bytes) {
  return (bytes / 1048576).toFixed(bytes >= 104857600 ? 0 : 1) + ' MB';
}

/** 166.1 -> "2m 46s" */
function fmtSecs(s) {
  return s >= 60 ? `${Math.floor(s / 60)}m ${Math.round(s % 60)}s` : `${s.toFixed(1)}s`;
}

/**
 * Rows of `name | bar(value) | share%`, capped at `top`.
 * @param {{name: string, value: number}[]} items
 * @param {(v: number) => string} fmt
 */
function barTable(items, fmt, top, totalOverride) {
  const shown = items.slice(0, top);
  const max = shown.reduce((m, r) => Math.max(m, r.value), 1);
  const total = totalOverride || items.reduce((s, r) => s + r.value, 0) || 1;
  return shown.map((r, i) =>
    `<tr><td class="num rank">${i + 1}</td>` +
    `<td class="name">${escapeHtml(r.name)}</td>` +
    `<td class="bar-cell"><span class="bar" style="width:${(r.value / max * 100).toFixed(2)}%"></span>` +
    `<span class="bar-label">${escapeHtml(fmt(r.value))}</span></td>` +
    `<td class="num">${(r.value / total * 100).toFixed(1)}%</td></tr>`).join('\n');
}

/** @param {ReturnType<typeof buildBuildReport>} build */
function renderBuildSection(build) {
  if (!build) {
    return `<h2>Build report</h2>
  <p class="subtitle">No release binary found — run <code>environment/build.sh udev release</code>
  (with <code>cargo build --timings</code> for compile-time data), then regenerate this report.</p>`;
  }

  const t = build.timings;
  const text = build.sections?.find(s => s.name === '.text');
  const tiles = [
    [fmtMB(build.binBytes), 'binary size (unstripped)'],
    text ? [fmtMB(text.size), '.text (code)'] : null,
    t ? [fmtSecs(t.wallSeconds), 'clean-build wall time'] : null,
    t ? [fmtSecs(t.cpuSeconds), 'total CPU time'] : null,
    t ? [String(t.unitCount), 'compilation units'] : null,
  ].filter(Boolean).map(([v, k]) =>
    `<div class="tile"><div class="v">${escapeHtml(v)}</div><div class="k">${escapeHtml(k)}</div></div>`).join('\n    ');

  const flags = `<p class="subtitle">Flags for <code>build.sh udev release</code>:
  cargo <b>release</b> profile ${build.profileOverrides.length
    ? `overridden in the bin workspace root: <code>${escapeHtml(build.profileOverrides.join(', '))}</code>; defaults otherwise`
    : 'with no overrides'}
  (defaults: opt-level=3, thin-local LTO, codegen-units=16, panic=unwind, no debug info, not stripped),
  features <code>--no-default-features --features backend-native</code>,
  rustflags from <code>.cargo/config.toml</code>: <code>${escapeHtml(build.rustflags.join(' ') || '(none)')}</code>.</p>`;

  const parts = [`<h2>Build report — release binary</h2>`, flags, `<div class="tiles">\n    ${tiles}\n  </div>`];

  if (t) {
    parts.push(`<h2>Slowest crates to compile</h2>
  <p class="subtitle">CPU seconds per crate summed over its units (build script + lib + bin),
  from the cargo --timings report of ${escapeHtml(t.generatedAt)}. Wall time is lower — units compile in parallel.</p>
  <div class="table-wrap"><table>
    <thead><tr><th class="num">#</th><th>crate</th><th>compile CPU time</th><th class="num">% of total</th></tr></thead>
    <tbody>\n${barTable(t.crates.map(c => ({ name: c.name, value: c.cpuSeconds })), v => v.toFixed(1) + 's', 40)}\n</tbody>
  </table></div>`);
  }

  if (build.textByCrate) {
    parts.push(`<h2>Binary code size by crate</h2>
  <p class="subtitle">.text bytes attributed via nm symbol names. Monomorphized generics count
  toward the crate that <em>defines</em> them — core/alloc/std/hashbrown are large because every
  crate instantiates them, not because of their own code.</p>
  <div class="table-wrap"><table>
    <thead><tr><th class="num">#</th><th>crate</th><th>attributed code size</th><th class="num">% of code</th></tr></thead>
    <tbody>\n${barTable(build.textByCrate.map(c => ({ name: c.name, value: c.bytes })), fmtMB, 40)}\n</tbody>
  </table></div>`);
  }

  if (build.sections) {
    parts.push(`<h2>Binary anatomy (ELF sections &gt; 1 MB)</h2>
  <div class="table-wrap"><table>
    <thead><tr><th class="num">#</th><th>section</th><th>size</th><th class="num">% of file</th></tr></thead>
    <tbody>\n${barTable(build.sections.filter(s => s.size > 1048576).map(s => ({ name: s.name, value: s.size })), fmtMB, 20, build.binBytes)}\n</tbody>
  </table></div>`);
  }

  if (build.rlibs.length) {
    parts.push(`<h2>Largest compiled artifacts (.rlib)</h2>
  <p class="subtitle">Compile/disk cost per crate, NOT final-binary share — the linker drops
  unused code (ash is huge here but small in the binary).</p>
  <div class="table-wrap"><table>
    <thead><tr><th class="num">#</th><th>crate</th><th>rlib size</th><th class="num">% of deps</th></tr></thead>
    <tbody>\n${barTable(build.rlibs.map(r => ({ name: r.name, value: r.bytes })), fmtMB, 25)}\n</tbody>
  </table></div>`);
  }

  return parts.join('\n\n  ');
}

function renderHtml({ roots, rows }, build) {
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
<title>y5 workspace report — dependencies &amp; build</title>
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
  <h1>Workspace report</h1>
  <p class="subtitle">Dependency blast radius per crate (how many other crates recompile when it
  changes, sorted most-blasting first) plus, when a release build is present, binary size and
  compile-time breakdowns.</p>

  <div class="tiles">
    <div class="tile"><div class="v">${total}</div><div class="k">internal crates</div></div>
    <div class="tile"><div class="v">${roots.length}</div><div class="k">workspace roots</div></div>
    <div class="tile"><div class="v">${maxBlast}</div><div class="k">max blast radius</div></div>
    <div class="tile"><div class="v">${avgBlast.toFixed(1)}</div><div class="k">avg blast radius</div></div>
  </div>

  ${renderBuildSection(build)}

  <h2>Blast radius per crate</h2>
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
  const build = buildBuildReport(model.crates);

  if (argv.includes('--json')) {
    process.stdout.write(JSON.stringify({
      generatedFor: path.basename(REPO_ROOT),
      crateCount: model.rows.length,
      workspaceRoots: model.roots.map(r => path.relative(REPO_ROOT, r)),
      crates: model.rows,
      build: build && {
        binary: { path: path.relative(REPO_ROOT, build.binPath), bytes: build.binBytes, mtime: build.binMtime },
        rustflags: build.rustflags,
        profileOverrides: build.profileOverrides,
        sections: build.sections,
        textByCrate: build.textByCrate,
        rlibs: build.rlibs,
        timings: build.timings,
      },
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
  fs.writeFileSync(out, renderHtml(model, build), 'utf-8');
  console.log(`Analyzed ${model.rows.length} crates across ${model.roots.length} workspace roots.`);
  console.log(build
    ? `Build section: ${path.relative(REPO_ROOT, build.binPath)} (${fmtMB(build.binBytes)})`
    : 'Build section: skipped (no release binary found)');
  console.log(`Report written to: ${out}`);
}

main();
