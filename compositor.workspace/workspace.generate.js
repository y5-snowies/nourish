// Generate every Cargo.toml in the linked tree from the authored sources.
//
//   vendor.catalog.json      external crates: exact version, features, vendored
//                            path, per-workspace overrides, [patch.crates-io]
//   workspace.catalog.json   each root's members/resolver/link-features, and
//                            every per-crate manifest fact that is not a dep name
//   <crate>/crate.json       that crate's dependency names, and nothing else
//
// Absorbs the old `workspace.link.js`: the internal cross-workspace `[path]`
// links it used to splice into a sentinel block are now just part of the
// generated root manifest, so there is no block to go stale and nothing for a
// drift check to catch.
//
// The manifests are build artifacts. They are gitignored, and `environment/build.sh`
// and `environment/check.sh` run this before invoking cargo, so a fresh clone
// bootstraps itself.
//
// Usage:
//   node workspace.generate.js            write the manifests + the .gitignore block
//   node workspace.generate.js --check    write nothing; fail if anything would change
//   node workspace.generate.js --stdout <crate-dir>   print one manifest
//   node workspace.generate.js --shape [crate|root]   what does this tree depend on?
//   node workspace.generate.js --clean                delete every generated manifest + lock

'use strict';

const fs = require('fs');
const path = require('path');
const jsonc = require('./jsonc.js');

// The tooling lives in compositor.workspace/; the tree it generates is its parent.
const HERE = __dirname;
const REPO = path.dirname(__dirname);
const CHECK = process.argv.includes('--check');
const STDOUT = process.argv.indexOf('--stdout');
const SHAPE = process.argv.indexOf('--shape');
const CLEAN = process.argv.includes('--clean');

const DEFAULT_VERSION = '0.0.1';
const DEFAULT_EDITION = '2024';
const DEFAULT_LIB = 'lib.rs';

const IGNORE_START = '# --- GENERATED MANIFEST IGNORES START ---';
const IGNORE_END = '# --- GENERATED MANIFEST IGNORES END ---';

// ---------------------------------------------------------------------------
// TOML emission
// ---------------------------------------------------------------------------

const str = (s) => JSON.stringify(String(s));

function value(v) {
    if (Array.isArray(v)) return `[${v.map(value).join(', ')}]`;
    if (v && typeof v === 'object') {
        const inner = Object.entries(v)
            .map(([k, x]) => `${k} = ${value(x)}`)
            .join(', ');
        return `{ ${inner} }`;
    }
    if (typeof v === 'boolean' || typeof v === 'number') return String(v);
    return str(v);
}

/// A dependency spec from the catalog, as an inline table, with `path` rewritten
/// from repo-relative to relative-from-this-root.
function depSpec(spec, fromDir, extra) {
    const parts = [];
    if (spec.path !== undefined) {
        parts.push(`path = ${str(rel(fromDir, path.join(REPO, spec.path)))}`);
    }
    if (spec.version !== undefined) parts.push(`version = ${str(spec.version)}`);
    if (spec['default-features'] !== undefined) {
        parts.push(`default-features = ${spec['default-features']}`);
    }
    if (spec.features !== undefined) parts.push(`features = ${value(spec.features)}`);
    for (const [k, v] of Object.entries(extra || {})) parts.push(`${k} = ${value(v)}`);
    return `{ ${parts.join(', ')} }`;
}

const rel = (from, to) => {
    const r = path.relative(from, to).replace(/\\/g, '/');
    return r === '' ? '.' : r;
};

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

function load() {
    const vendor = jsonc.read(path.join(HERE, 'vendor.catalog.json'));
    const workspace = jsonc.read(path.join(HERE, 'workspace.catalog.json'));

    for (const [key, over] of Object.entries(vendor.workspace || {})) {
        for (const name of Object.keys(over)) {
            if (!(name in vendor.crates)) {
                throw new Error(
                    `vendor.catalog.json: workspace override ${key} names ${name}, ` +
                        `which is not in "crates". An override may only override.`,
                );
            }
        }
    }
    for (const [name, spec] of Object.entries(vendor.crates)) {
        if (spec.preset !== undefined && !vendor.presets?.[name]?.[spec.preset]) {
            throw new Error(`vendor.catalog.json: ${name} names preset ${str(spec.preset)}, which is not defined`);
        }
        if (spec.version === undefined) continue;

        // EXACTNESS. This is the whole reason the lockfiles can be gitignored: the
        // catalog IS the pin for every direct dependency. Cargo reads a bare
        // `"1.2.3"` as `^1.2.3` — which is how `zbus = "5.15.0"` was silently
        // building against 5.19.0 — so a pin has to be written `=1.2.3` and is
        // rejected otherwise. A genuine range must say `"mode": "predicate"` out loud.
        //
        // Vendored forks are exempt: their `path` is what cargo resolves, and the
        // version merely mirrors the fork's own `[package] version`.
        if (spec.path !== undefined || spec.mode === 'predicate') continue;
        if (!/^=\d+\.\d+\.\d+/.test(spec.version)) {
            throw new Error(
                `vendor.catalog.json: ${name} = ${str(spec.version)} is not an exact pin. ` +
                    `Write "=x.y.z", or set "mode": "predicate" if a range is genuinely wanted.`,
            );
        }
    }
    return { vendor, workspace };
}

/// Resolve one external crate for one root: base spec, then a container-level
/// override, then an exact-root override. A layer may name a `preset`, which
/// replaces the feature selection wholesale.
///
/// Presets exist because the feature sets repeat: thirteen roots wanted the same
/// seventeen smithay features, and writing that list thirteen times is how the
/// three "different" sets came to differ only in their ORDER.
function resolveSpec(vendor, name, rootRel) {
    const container = rootRel.includes('/') ? rootRel.split('/')[0] : null;
    let spec = vendor.crates[name];
    if (spec === undefined) throw new Error(`${rootRel}: ${name} is not in vendor.catalog.json`);
    const apply = (layer) => {
        if (!layer) return;
        if (layer.preset !== undefined) {
            const p = vendor.presets?.[name]?.[layer.preset];
            if (!p) throw new Error(`${rootRel}: ${name} names preset ${str(layer.preset)}, which is not defined`);
            // A preset REPLACES the selection; it never merges with the previous one.
            delete spec.features;
            delete spec['default-features'];
            spec = { ...spec, ...p };
        }
        const { preset, ...rest } = layer;
        void preset;
        spec = { ...spec, ...rest };
    };
    spec = { ...spec };
    const own = spec.preset;
    delete spec.preset;
    apply(own !== undefined ? { preset: own } : null);
    apply(container && vendor.workspace?.[container]?.[name]);
    apply(vendor.workspace?.[rootRel]?.[name]);
    delete spec.mode;
    return spec;
}

function crateDirs(rootRel, members) {
    const out = [];
    for (const g of members) {
        for (const m of fs.globSync(path.join(g, 'crate.json'), { cwd: path.join(REPO, rootRel) })) {
            out.push(path.dirname(m).replace(/\\/g, '/'));
        }
    }
    return out.sort();
}

/// The prefix a child directory must repeat from its parent: the parent's name
/// minus its first dot-segment, plus a dot. Mirrors `chainPrefix` in
/// workspace.lint.js — the two must agree or the lint would reject what the
/// generator emits.
function chainPrefix(parentDirName) {
    const parts = parentDirName.split('.');
    return (parts.length > 1 ? parts.slice(1).join('.') : parentDirName) + '.';
}

/// The chain a root contributes. A root named `compositor.<x>` starts fresh
/// (`compositor.expansion/compositor.y5` -> `compositor.y5`); any other root
/// merges through its container (`compositor.support/support.smithay` ->
/// `compositor.support.smithay`).
function rootChain(rootRel) {
    const parts = rootRel.split('/');
    const tail = parts[parts.length - 1];
    if (parts.length === 1 || tail.startsWith('compositor.')) return tail;
    return `${parts[0]}.${tail.split('.').slice(1).join('.')}`;
}

/// The package name IS the directory chain, dots turned into underscores.
///
/// Each path component repeats its parent's chain prefix, and only the remainder
/// is new — which is why `state.xdg.activation/xdg.activation.dispatch` yields
/// `…state_xdg_activation_dispatch` and not `…activation_dispatch`. Segments are
/// not single words: strip the parent's actual prefix, never "the first dot-segment".
function packageName(rootRel, crateRel) {
    const chain = [rootChain(rootRel)];
    let parent = rootRel.split('/').pop();
    for (const seg of crateRel.split('/')) {
        const prefix = chainPrefix(parent);
        if (!seg.startsWith(prefix)) {
            throw new Error(
                `${rootRel}/${crateRel}: "${seg}" does not start with "${prefix}" — ` +
                    `the chain-prefix convention is broken, so the package name cannot be derived`,
            );
        }
        chain.push(seg.slice(prefix.length));
        parent = seg;
    }
    return chain.join('.').replace(/\./g, '_');
}

/// The deps a crate declares, per table, with the comments that belong above each.
function crateDeps(crateDir) {
    const text = fs.readFileSync(path.join(crateDir, 'crate.json'), 'utf-8');
    const json = jsonc.parse(text, path.join(crateDir, 'crate.json'));
    const out = {};
    for (const field of ['deps', 'dev', 'build']) {
        if (!json[field]) continue;
        const comments = jsonc.commentsFor(text, field);
        out[field] = json[field].map((name) => ({ name, comments: comments[name] || [] }));
    }
    out.keep = json.keep || {};
    return out;
}

// ---------------------------------------------------------------------------
// Manifests
// ---------------------------------------------------------------------------

function leafManifest({ rootRel, crateRel, crateDir, facts, deps }) {
    const L = [];
    L.push('# GENERATED by workspace.generate.js — edit crate.json / the root catalogs instead.');
    L.push('');
    L.push('[package]');
    L.push(`name = ${str(packageName(rootRel, crateRel))}`);
    L.push(`version = ${str(facts.version ?? DEFAULT_VERSION)}`);
    L.push(`edition = ${str(facts.edition ?? DEFAULT_EDITION)}`);
    if (facts.build_script !== undefined) L.push(`build = ${value(facts.build_script)}`);
    for (const [k, v] of Object.entries(facts.package || {})) {
        L.push(`${k} = ${str(k === 'readme' ? rel(crateDir, path.join(REPO, v)) : v)}`);
    }

    const optional = new Set(facts.optional || []);
    for (const [field, header] of [
        ['deps', 'dependencies'],
        ['dev', 'dev-dependencies'],
        ['build', 'build-dependencies'],
    ]) {
        if (!deps[field]) continue;
        L.push('');
        L.push(`[${header}]`);
        for (const { name, comments } of deps[field]) {
            for (const c of comments) L.push(`# ${c}`);
            const attrs = optional.has(name) ? ', optional = true' : '';
            L.push(`${name} = { workspace = true${attrs} }`);
        }
    }

    if (facts.lib !== false) {
        L.push('');
        L.push('[lib]');
        L.push(`path = ${str(facts.lib ?? DEFAULT_LIB)}`);
    }

    for (const bin of facts.bin || []) {
        L.push('');
        L.push('[[bin]]');
        for (const [k, v] of Object.entries(bin)) L.push(`${k} = ${value(v)}`);
    }

    if (facts.features) {
        L.push('');
        L.push('[features]');
        // Emitted verbatim: `dep:x` and `y?/feat` are strings to this generator,
        // never parsed. The feature graph stays exactly as authored.
        for (const [k, v] of Object.entries(facts.features)) L.push(`${k} = ${value(v)}`);
    }

    // `keep` names dependencies that ARE required but never appear in the crate's
    // source — spliced in by `tonic::include_proto!` or `include!(OUT_DIR)`, or
    // pulled in by an unhygienic macro that expands to a bare `smithay::` path.
    // Hand them to cargo-shear so the tool agrees with the lint instead of
    // reporting them forever.
    const keep = Object.keys(deps.keep || {});
    if (keep.length) {
        L.push('');
        L.push('[package.metadata.cargo-shear]');
        L.push(`ignored = ${value(keep)}`);
        for (const [name, why] of Object.entries(deps.keep)) L.push(`# ${name}: ${why}`);
    }

    return L.join('\n') + '\n';
}

function rootManifest({ rootRel, entry, vendor, crates, internalPaths }) {
    const rootDir = path.join(REPO, rootRel);
    const L = [];
    L.push('# GENERATED by workspace.generate.js — edit the root catalogs instead.');
    L.push('');
    L.push('[workspace]');
    L.push(`members = ${value(entry.members)}`);
    if (entry.resolver !== undefined) L.push(`resolver = ${str(entry.resolver)}`);

    // [patch.crates-io] — only the roots the catalog names.
    const patches = Object.entries(vendor.patch || {}).filter(([, p]) => p.roots.includes(rootRel));
    if (patches.length) {
        L.push('');
        L.push('[patch.crates-io]');
        for (const [name, p] of patches.sort()) {
            L.push(`${name} = { path = ${str(rel(rootDir, path.join(REPO, p.path)))} }`);
        }
    }

    L.push('');
    L.push('[workspace.dependencies]');

    // Externals: the UNION of what this root's member crates actually declare, so
    // a root can never carry a dependency none of its crates uses. Root
    // minimality is a property of the generator, not something a lint has to police.
    const wanted = new Set();
    for (const c of crates) {
        for (const field of ['deps', 'dev', 'build']) {
            for (const d of c.deps[field] || []) {
                if (!d.name.startsWith('compositor_')) wanted.add(d.name);
            }
        }
    }
    for (const name of [...wanted].sort()) {
        L.push(`${name} = ${depSpec(resolveSpec(vendor, name, rootRel), rootDir)}`);
    }

    // Internal path links, with the root's feature overrides.
    if (internalPaths.size) {
        L.push('');
        for (const name of [...internalPaths.keys()].sort()) {
            const ov = entry.links?.[name];
            const extra = {};
            if (ov?.['default-features'] === false) extra['default-features'] = false;
            if (Array.isArray(ov?.features)) extra.features = ov.features;
            L.push(
                `${name} = ${depSpec({ path: rel(REPO, internalPaths.get(name)) }, rootDir, extra)}`,
            );
        }
    }

    for (const [header, body] of Object.entries(entry.profiles || defaultProfiles(rootRel))) {
        L.push('');
        L.push(`[${header}]`);
        for (const [k, v] of Object.entries(body)) L.push(`${k} = ${value(v)}`);
    }

    return L.join('\n') + '\n';
}

/// The profiles every root carries. `release-fast` is what every local build and
/// `environment/check.sh` use; only the loader overrides `release` with fat LTO,
/// because it is the only root that produces a shipped binary.
function defaultProfiles(rootRel) {
    const fast = {
        'profile.release-fast': { inherits: 'release', lto: false, 'codegen-units': 16 },
    };
    if (rootRel !== 'compositor.kernel/kernel.loader') return fast;
    return {
        'profile.release': {
            strip: 'symbols',
            lto: 'fat',
            'codegen-units': 1,
            debug: 'line-tables-only',
            'split-debuginfo': 'packed',
        },
        ...fast,
    };
}

// ---------------------------------------------------------------------------
// Drive
// ---------------------------------------------------------------------------

/// Delete every generated manifest and lockfile.
///
/// Driven by the same root list that generates them, so it can only ever remove
/// what this script produces — never a hand-written manifest in `vendor/`, the
/// installer, `developer.tool` or the toolkit. `find . -name Cargo.toml -delete`
/// would take those with it.
///
/// Lockfiles go too: they are cargo's output for a generated input, so keeping
/// them across a clean would defeat the point.
function clean() {
    const { workspace } = load();
    let n = 0;
    for (const rootRel of Object.keys(workspace.roots)) {
        const targets = [
            path.join(REPO, rootRel, 'Cargo.toml'),
            path.join(REPO, rootRel, 'Cargo.lock'),
            ...fs
                .globSync(path.join(rootRel, '*/*/*/Cargo.toml'), { cwd: REPO })
                .map((f) => path.join(REPO, f)),
        ];
        for (const f of targets) {
            if (!fs.existsSync(f)) continue;
            fs.unlinkSync(f);
            n++;
        }
    }
    console.error(`workspace.generate --clean: removed ${n} generated file(s)`);
    console.error('  the next build.sh / check.sh regenerates them');
}

function main() {
    if (SHAPE !== -1) return shape(process.argv[SHAPE + 1]);
    if (CLEAN) return clean();
    const { vendor, workspace } = load();
    const roots = Object.keys(workspace.roots).sort();

    // Every internal crate in the repo, by package name -> absolute dir. Generation
    // is GLOBAL: any root may link any crate.
    const internalPaths = new Map();
    const perRoot = new Map();
    for (const rootRel of roots) {
        const entry = workspace.roots[rootRel];
        const list = crateDirs(rootRel, entry.members).map((crateRel) => {
            const crateDir = path.join(REPO, rootRel, crateRel);
            const name = packageName(rootRel, crateRel);
            internalPaths.set(name, crateDir);
            return {
                rootRel,
                crateRel,
                crateDir,
                name,
                facts: entry.crates?.[crateRel] || {},
                deps: crateDeps(crateDir),
            };
        });
        perRoot.set(rootRel, list);
    }

    if (STDOUT !== -1) {
        const want = path.resolve(process.argv[STDOUT + 1]);
        for (const list of perRoot.values()) {
            for (const c of list) if (path.resolve(c.crateDir) === want) return void process.stdout.write(leafManifest(c));
        }
        throw new Error(`no crate at ${want}`);
    }

    const files = new Map();
    for (const rootRel of roots) {
        const list = perRoot.get(rootRel);
        // A root links only what its crates actually name.
        const linked = new Map();
        for (const c of list) {
            for (const field of ['deps', 'dev', 'build']) {
                for (const d of c.deps[field] || []) {
                    if (d.name.startsWith('compositor_')) {
                        if (!internalPaths.has(d.name)) {
                            throw new Error(`${c.crateRel}: unknown internal crate ${d.name}`);
                        }
                        linked.set(d.name, internalPaths.get(d.name));
                    }
                }
            }
        }
        files.set(
            path.join(REPO, rootRel, 'Cargo.toml'),
            rootManifest({ rootRel, entry: workspace.roots[rootRel], vendor, crates: list, internalPaths: linked }),
        );
        for (const c of list) files.set(path.join(c.crateDir, 'Cargo.toml'), leafManifest(c));
    }

    files.set(path.join(REPO, '.gitignore'), gitignore(roots));

    let changed = 0;
    for (const [file, body] of files) {
        const old = fs.existsSync(file) ? fs.readFileSync(file, 'utf-8') : null;
        if (old === body) continue;
        changed++;
        if (CHECK) console.error(`would change: ${path.relative(REPO, file)}`);
        else fs.writeFileSync(file, body);
    }

    if (CHECK) {
        if (changed) {
            console.error(`workspace.generate --check: ${changed} file(s) out of date`);
            process.exit(1);
        }
        console.error('workspace.generate --check: up to date');
        return;
    }
    console.error(`workspace.generate: ${files.size} file(s), ${changed} written`);
}

/// The ignore block, generated from the same root list that drives generation, so
/// it can never name something the generator does not produce. EXACT paths only —
/// a bare `Cargo.toml` pattern would also swallow vendor/ (206 manifests), the
/// installer, developer.tool, y5.template and the toolkit, all of which are
/// hand-written and must stay tracked.
function gitignore(roots) {
    const file = path.join(REPO, '.gitignore');
    const text = fs.readFileSync(file, 'utf-8');
    const lines = [IGNORE_START];
    for (const r of roots) {
        lines.push(`${r}/Cargo.toml`);
        lines.push(`${r}/Cargo.lock`);
        lines.push(`${r}/*/*/*/Cargo.toml`);
    }
    lines.push(IGNORE_END);
    const block = lines.join('\n');

    const re = new RegExp(`\\n*${IGNORE_START}[\\s\\S]*?${IGNORE_END}`, 'g');
    const base = text.replace(re, '').trimEnd();
    return `${base}\n\n${block}\n`;
}

// ---------------------------------------------------------------------------
// --shape: what does this tree actually depend on?
// ---------------------------------------------------------------------------

/// Print the dependency shape. Lives here rather than in its own script because it
/// answers from the SAME resolution the manifests are generated from — a separate
/// reporter would eventually describe a tree that is not the one being built.
///
///   --shape             the whole tree
///   --shape <crate>     one external crate: version, and who selects which preset
///   --shape <root>      one workspace root: its externals and its internal links
function shape(focus) {
    const { vendor, workspace } = load();
    const roots = Object.keys(workspace.roots).sort();

    // root -> { crates, externals: Map(name -> spec), internals: Set }
    const per = new Map();
    let totalCrates = 0;
    let totalEdges = 0;
    for (const rootRel of roots) {
        const entry = workspace.roots[rootRel];
        const externals = new Map();
        const internals = new Set();
        let n = 0;
        for (const crateRel of crateDirs(rootRel, entry.members)) {
            n++;
            const deps = crateDeps(path.join(REPO, rootRel, crateRel));
            for (const field of ['deps', 'dev', 'build']) {
                for (const d of deps[field] || []) {
                    totalEdges++;
                    if (d.name.startsWith('compositor_')) internals.add(d.name);
                    else externals.set(d.name, resolveSpec(vendor, d.name, rootRel));
                }
            }
        }
        totalCrates += n;
        per.set(rootRel, { crates: n, externals, internals });
    }

    // Which preset each root ends up with, per crate.
    const presetOf = (name, rootRel) => {
        const want = per.get(rootRel).externals.get(name);
        if (!want) return null;
        for (const [pn, p] of Object.entries(vendor.presets?.[name] || {})) {
            const same =
                JSON.stringify([...(p.features || [])].sort()) === JSON.stringify([...(want.features || [])].sort()) &&
                (p['default-features'] ?? null) === (want['default-features'] ?? null);
            if (same) return pn;
        }
        return null;
    };

    const pad = (s, n) => String(s).padEnd(n);
    const versionOf = (s) => (s.path ? `vendor ${s.version || ''}`.trim() : s.version || '—');

    if (focus && vendor.crates[focus]) {
        const spec = vendor.crates[focus];
        console.log(`${focus}  ${versionOf(spec)}${spec.mode ? `  (mode: ${spec.mode})` : ''}`);
        if (spec.path) console.log(`  path      ${spec.path}`);
        const usersByPreset = new Map();
        for (const r of roots) {
            if (!per.get(r).externals.has(focus)) continue;
            const p = presetOf(focus, r) || '(no preset)';
            if (!usersByPreset.has(p)) usersByPreset.set(p, []);
            usersByPreset.get(p).push(r);
        }
        for (const [p, rs] of [...usersByPreset].sort((a, b) => b[1].length - a[1].length)) {
            const feats = vendor.presets?.[focus]?.[p]?.features || spec.features;
            console.log(`\n  ${p}  (${rs.length} root${rs.length === 1 ? '' : 's'})`);
            if (feats?.length) console.log(`    features: ${feats.join(', ')}`);
            for (const r of rs) console.log(`      ${r}`);
        }
        return;
    }

    if (focus && per.has(focus)) {
        const info = per.get(focus);
        console.log(`${focus}\n  ${info.crates} crates, ${info.externals.size} external, ${info.internals.size} internal links\n`);
        console.log('  EXTERNAL');
        for (const name of [...info.externals.keys()].sort()) {
            const p = presetOf(name, focus);
            console.log(`    ${pad(name, 24)} ${pad(versionOf(info.externals.get(name)), 16)}${p ? `preset: ${p}` : ''}`);
        }
        return;
    }

    if (focus) {
        console.error(`--shape: "${focus}" is neither a catalog crate nor a workspace root`);
        process.exitCode = 1;
        return;
    }

    console.log(`y5 workspace shape`);
    console.log(`  ${roots.length} roots · ${totalCrates} crates · ${totalEdges} dependency edges`);
    console.log(`  ${Object.keys(vendor.crates).length} external crates · ${Object.keys(vendor.presets || {}).length} with feature presets\n`);

    console.log(`EXTERNAL`);
    console.log(`  ${pad('crate', 24)}${pad('version', 18)}${pad('roots', 7)}presets`);
    for (const name of Object.keys(vendor.crates).sort()) {
        const users = roots.filter((r) => per.get(r).externals.has(name));
        const counts = new Map();
        for (const r of users) {
            const p = presetOf(name, r);
            if (p) counts.set(p, (counts.get(p) || 0) + 1);
        }
        const ps = [...counts].sort((a, b) => b[1] - a[1]).map(([p, n]) => `${p}×${n}`).join(' ');
        console.log(`  ${pad(name, 24)}${pad(versionOf(vendor.crates[name]), 18)}${pad(users.length, 7)}${ps}`);
    }

    console.log(`\nROOTS`);
    console.log(`  ${pad('root', 46)}${pad('crates', 8)}${pad('ext', 5)}internal links`);
    for (const r of roots) {
        const i = per.get(r);
        console.log(`  ${pad(r, 46)}${pad(i.crates, 8)}${pad(i.externals.size, 5)}${i.internals.size}`);
    }
    console.log(`\n  (--shape <crate>  or  --shape <root>  for detail)`);
}

/// Build every manifest in memory, without writing. Used by the migration's
/// fidelity check.
function build() {
    const { vendor, workspace } = load();
    const roots = Object.keys(workspace.roots).sort();
    const internalPaths = new Map();
    const perRoot = new Map();
    for (const rootRel of roots) {
        const entry = workspace.roots[rootRel];
        const list = crateDirs(rootRel, entry.members).map((crateRel) => {
            const crateDir = path.join(REPO, rootRel, crateRel);
            const name = packageName(rootRel, crateRel);
            internalPaths.set(name, crateDir);
            return { rootRel, crateRel, crateDir, name, facts: entry.crates?.[crateRel] || {}, deps: crateDeps(crateDir) };
        });
        perRoot.set(rootRel, list);
    }
    const files = new Map();
    for (const rootRel of roots) {
        const list = perRoot.get(rootRel);
        const linked = new Map();
        for (const c of list) {
            for (const field of ['deps', 'dev', 'build']) {
                for (const d of c.deps[field] || []) {
                    if (d.name.startsWith('compositor_')) {
                        if (!internalPaths.has(d.name)) throw new Error(`${c.crateRel}: unknown internal crate ${d.name}`);
                        linked.set(d.name, internalPaths.get(d.name));
                    }
                }
            }
        }
        files.set(
            path.join(REPO, rootRel, 'Cargo.toml'),
            rootManifest({ rootRel, entry: workspace.roots[rootRel], vendor, crates: list, internalPaths: linked }),
        );
        for (const c of list) files.set(path.join(c.crateDir, 'Cargo.toml'), leafManifest(c));
    }
    return files;
}

module.exports = { build, packageName, REPO };

if (require.main === module) main();
