#!/usr/bin/env node
// cover/lib/lcov-merge.js — merge lcov files at the lcov level (by file / line /
// function name), taking the max execution count per line and per function. Used to
// combine the empty baseline (every member at 0%) with the tests report (executed
// counts) into the final report, so the baseline stays a pristine, separate artifact
// and covmap-hash differences between the two builds never matter.
//
// Usage: node lcov-merge.js baseline.lcov [tests.lcov ...]  > merged.lcov

const fs = require('fs');

// Functions are keyed by their START LINE, not by mangled name: rustc's mangled names
// carry a per-build hash, so the same function has different names in the baseline build
// vs the test build. Keying by line (stable across builds, same source) makes the merged
// denominator equal the baseline's — tests only flip a function/line from 0 to hit, they
// never inflate the total. The baseline is passed first, so it supplies the display name.
//
// file -> { fnName: Map<line,name>, fnCount: Map<line,count>, da: Map<line,count> }
const files = new Map();
function get(f) {
  if (!files.has(f)) files.set(f, { fnName: new Map(), fnCount: new Map(), da: new Map() });
  return files.get(f);
}

for (const path of process.argv.slice(2)) {
  let text;
  try { text = fs.readFileSync(path, 'utf8'); } catch { continue; }
  let cur = null, nameLine = null;   // nameLine: FN name -> start line, for this record
  for (const line of text.split('\n')) {
    if (line.startsWith('SF:')) { cur = get(line.slice(3)); nameLine = new Map(); }
    else if (!cur) continue;
    else if (line.startsWith('FN:')) {
      const c = line.slice(3).indexOf(',');
      if (c > 0) {
        const ln = +line.slice(3, 3 + c) || 0, name = line.slice(3 + c + 1);
        nameLine.set(name, ln);
        if (!cur.fnName.has(ln)) cur.fnName.set(ln, name);   // first input (baseline) names it
        if (!cur.fnCount.has(ln)) cur.fnCount.set(ln, 0);
      }
    } else if (line.startsWith('FNDA:')) {
      const c = line.slice(5).indexOf(',');
      if (c > 0) {
        const cnt = +line.slice(5, 5 + c) || 0, ln = nameLine.get(line.slice(5 + c + 1));
        if (ln !== undefined) cur.fnCount.set(ln, Math.max(cur.fnCount.get(ln) || 0, cnt));
      }
    } else if (line.startsWith('DA:')) {
      const c = line.slice(3).indexOf(',');
      if (c > 0) {
        const ln = +line.slice(3, 3 + c), cnt = +line.slice(3 + c + 1) || 0;
        cur.da.set(ln, Math.max(cur.da.get(ln) || 0, cnt));
      }
    } else if (line.startsWith('end_of_record')) { cur = null; nameLine = null; }
  }
}

const out = [];
for (const f of [...files.keys()].sort()) {
  const r = files.get(f);
  out.push('SF:' + f);
  const fnLines = [...r.fnName.keys()].sort((a, b) => a - b);
  for (const ln of fnLines) out.push(`FN:${ln},${r.fnName.get(ln)}`);
  let fnh = 0;
  for (const ln of fnLines) { const c = r.fnCount.get(ln) || 0; out.push(`FNDA:${c},${r.fnName.get(ln)}`); if (c > 0) fnh++; }
  out.push(`FNF:${fnLines.length}`);
  out.push(`FNH:${fnh}`);
  let lh = 0;
  for (const ln of [...r.da.keys()].sort((a, b) => a - b)) { const c = r.da.get(ln); out.push(`DA:${ln},${c}`); if (c > 0) lh++; }
  out.push(`LF:${r.da.size}`);
  out.push(`LH:${lh}`);
  out.push('end_of_record');
}
process.stdout.write(out.join('\n') + (out.length ? '\n' : ''));
