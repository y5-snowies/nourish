#!/usr/bin/env node
// cover/lib/report.js — render the per-project lcov.info files into one self-contained
// HTML page (cover/report.html). No server, no deps: the coverage data is embedded, so it
// opens directly via file:// and works for the 0% baseline and for real coverage alike.
//
// The file list is shown as a collapsible PACKAGE/CRATE TREE: the crate hierarchy is the
// directory hierarchy (crates are flat — a dir holding lib.rs), so a path tree with
// per-node coverage aggregation + single-child path compression groups files by crate.

const fs = require('fs');
const path = require('path');

const COVER_ROOT = path.resolve(__dirname, '..');
const REPO_ROOT = path.resolve(COVER_ROOT, '..');
const OUT = path.join(COVER_ROOT, 'report.html');

const projJson = JSON.parse(fs.readFileSync(path.join(COVER_ROOT, 'projects.json'), 'utf8'));
const projects = Object.keys(projJson.projects || {});

function parseLcov(text) {
  const files = [];
  let cur = null;
  for (const line of text.split('\n')) {
    if (line.startsWith('SF:')) cur = { path: line.slice(3), fnf: 0, fnh: 0, lf: 0, lh: 0 };
    else if (!cur) continue;
    else if (line.startsWith('FNF:')) cur.fnf = +line.slice(4) || 0;
    else if (line.startsWith('FNH:')) cur.fnh = +line.slice(4) || 0;
    else if (line.startsWith('LF:')) cur.lf = +line.slice(3) || 0;
    else if (line.startsWith('LH:')) cur.lh = +line.slice(3) || 0;
    else if (line.startsWith('end_of_record')) { files.push(cur); cur = null; }
  }
  return files;
}
const rel = p => (p.startsWith(REPO_ROOT + '/') ? p.slice(REPO_ROOT.length + 1) : p);

const data = [];
for (const proj of projects) {
  const lcov = path.join(COVER_ROOT, proj, 'report', 'lcov.info');
  if (!fs.existsSync(lcov)) { data.push({ project: proj, missing: true, files: [], totals: {} }); continue; }
  const files = parseLcov(fs.readFileSync(lcov, 'utf8')).map(f => ({ ...f, path: rel(f.path) }));
  const totals = files.reduce((a, f) => ({
    files: a.files + 1, fnf: a.fnf + f.fnf, fnh: a.fnh + f.fnh, lf: a.lf + f.lf, lh: a.lh + f.lh,
  }), { files: 0, fnf: 0, fnh: 0, lf: 0, lh: 0 });
  const mtime = fs.statSync(lcov).mtime.toISOString().replace('T', ' ').slice(0, 19);
  data.push({ project: proj, missing: false, files, totals, mtime });
}

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>y5 coverage</title>
<style>
:root{
  --bg:#f7f8fa; --panel:#fff; --ink:#1c2024; --muted:#6b7280; --line:#e6e8eb;
  --bar:#e9ecf0; --lo:#e5484d; --mid:#f5a623; --hi:#30a46c; --accent:#3b82f6; --row:#00000008;
}
@media (prefers-color-scheme:dark){:root{
  --bg:#0f1115; --panel:#171a21; --ink:#e6e8eb; --muted:#9aa4b2; --line:#262b33;
  --bar:#20242c; --lo:#e5484d; --mid:#ffb224; --hi:#46a758; --accent:#5b9dff; --row:#ffffff08;
}}
:root[data-theme=light]{--bg:#f7f8fa;--panel:#fff;--ink:#1c2024;--muted:#6b7280;--line:#e6e8eb;--bar:#e9ecf0;--row:#00000008;}
:root[data-theme=dark]{--bg:#0f1115;--panel:#171a21;--ink:#e6e8eb;--muted:#9aa4b2;--line:#262b33;--bar:#20242c;--row:#ffffff08;}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif}
.wrap{max-width:1100px;margin:0 auto;padding:26px 20px 60px}
header{display:flex;align-items:baseline;gap:12px;flex-wrap:wrap;margin-bottom:2px}
h1{font-size:20px;margin:0;font-weight:650}
.sub{color:var(--muted);font-size:12.5px}
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(210px,1fr));gap:12px;margin:18px 0 20px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:14px 16px}
.card h2{margin:0 0 2px;font-size:12px;font-weight:650;letter-spacing:.03em;text-transform:uppercase;color:var(--muted)}
.card .big{font-size:24px;font-weight:700;margin:4px 0 2px}
.card .meta{font-size:12px;color:var(--muted)}
.metric{display:flex;justify-content:space-between;font-size:12px;margin-top:7px;color:var(--muted)}
.bar{height:6px;border-radius:6px;background:var(--bar);overflow:hidden;margin-top:4px}
.bar>i{display:block;height:100%;border-radius:6px}
.toolbar{display:flex;gap:8px;align-items:center;flex-wrap:wrap;margin:6px 0 12px}
.tabs{display:flex;gap:6px;flex-wrap:wrap}
.tab{padding:6px 12px;border:1px solid var(--line);border-radius:999px;background:var(--panel);color:var(--ink);cursor:pointer;font-size:13px}
.tab.active{background:var(--accent);border-color:var(--accent);color:#fff}
.btn{padding:6px 11px;border:1px solid var(--line);border-radius:8px;background:var(--panel);color:var(--ink);cursor:pointer;font-size:12.5px}
.btn:hover{border-color:var(--accent)}
input[type=search]{margin-left:auto;padding:7px 11px;border:1px solid var(--line);border-radius:8px;background:var(--panel);color:var(--ink);min-width:180px;font-size:13px}
.tree{background:var(--panel);border:1px solid var(--line);border-radius:12px;overflow:hidden}
.hd{display:flex;align-items:center;gap:10px;padding:7px 14px;border-bottom:1px solid var(--line);color:var(--muted);font-size:11px;font-weight:600;letter-spacing:.03em;text-transform:uppercase}
.hd .hpct{width:74px;text-align:right}.hd .hcnt{width:70px;text-align:right}
.rows{max-height:70vh;overflow:auto}
.r{display:flex;align-items:center;gap:8px;padding:3px 14px;cursor:default;font-size:13px;border-bottom:1px solid transparent}
.r:hover{background:var(--row)}
.r .tw{width:14px;flex:0 0 14px;color:var(--muted);text-align:center;cursor:pointer;user-select:none;font-size:10px}
.r .nm{flex:1;min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.r.dir .nm{font-weight:600}
.r.file .nm{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}
.r .nm .seg{color:var(--muted);font-weight:400}
.badge{font-size:9.5px;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--accent);border:1px solid var(--accent);border-radius:4px;padding:0 4px;opacity:.8}
.r .cnt{width:70px;text-align:right;color:var(--muted);font-size:11.5px;font-variant-numeric:tabular-nums}
.r .pctw{width:120px;display:flex;align-items:center;gap:8px;justify-content:flex-end}
.r .pctw .bar{width:56px;margin:0}
.r .pctw b{width:44px;text-align:right;font-variant-numeric:tabular-nums;font-weight:600}
.empty{padding:26px;text-align:center;color:var(--muted)}
.themebtn{margin-left:6px}
</style>
</head>
<body>
<div class="wrap">
  <header>
    <h1>y5 coverage</h1>
    <span class="sub" id="gen"></span>
    <button class="btn themebtn" id="theme" title="Toggle theme">◐</button>
  </header>
  <div class="sub">LLVM source-based coverage · grouped by package/crate · generated by <code>cover/</code></div>

  <div class="cards" id="cards"></div>

  <div class="toolbar">
    <div class="tabs" id="tabs"></div>
    <button class="btn" id="expand">Expand all</button>
    <button class="btn" id="collapse">Collapse all</button>
    <input type="search" id="filter" placeholder="filter files…">
  </div>
  <div class="tree">
    <div class="hd"><span class="tw"></span><span style="flex:1">Package / file</span><span class="hcnt">lines</span><span class="hpct">line %</span></div>
    <div class="rows" id="rows"></div>
  </div>
  <div class="sub" id="foot" style="margin-top:8px"></div>
</div>
<script>
const DATA = ${JSON.stringify(data)};
const pct = (h,f)=> f? 100*h/f : 0;
const col = p=> p>=80?'var(--hi)':p>=40?'var(--mid)':'var(--lo)';
const bar = p=>'<div class="bar"><i style="width:'+p.toFixed(1)+'%;background:'+col(p)+'"></i></div>';

// ---- build a package/crate tree from file paths -------------------------------------
function buildTree(files){
  const root={name:'',path:'',dir:true,kids:new Map(),files:[],lf:0,lh:0,fnf:0,fnh:0,nf:0};
  for(const f of files){
    const segs=f.path.split('/'); let n=root;
    for(let i=0;i<segs.length-1;i++){
      const s=segs[i];
      if(!n.kids.has(s)) n.kids.set(s,{name:s,path:segs.slice(0,i+1).join('/'),dir:true,kids:new Map(),files:[],lf:0,lh:0,fnf:0,fnh:0,nf:0});
      n=n.kids.get(s);
    }
    n.files.push({name:segs[segs.length-1],path:f.path,dir:false,lf:f.lf,lh:f.lh,fnf:f.fnf,fnh:f.fnh});
  }
  (function agg(n){
    if(!n.dir){n.nf=1;return;}
    for(const c of n.kids.values()){agg(c);n.lf+=c.lf;n.lh+=c.lh;n.fnf+=c.fnf;n.fnh+=c.fnh;n.nf+=c.nf;}
    for(const f of n.files){n.lf+=f.lf;n.lh+=f.lh;n.fnf+=f.fnf;n.fnh+=f.fnh;n.nf++;}
  })(root);
  // compress single-child directory chains (no files, one child dir) into one row
  (function compress(n){
    for(const c of n.kids.values()){
      while(c.dir && c.files.length===0 && c.kids.size===1){
        const cc=[...c.kids.values()][0];
        c.name=c.name+'/'+cc.name; c.path=cc.path; c.kids=cc.kids; c.files=cc.files;
      }
      compress(c);
    }
  })(root);
  // a dir that directly holds source files is a crate (flat crate: lib.rs + one module)
  (function mark(n){ n.crate = n.dir && n.files.length>0; for(const c of n.kids.values()) mark(c); })(root);
  return root;
}
// ordered children: dirs (alpha) then files (alpha)
function kidsOf(n){
  const dirs=[...n.kids.values()].sort((a,b)=>a.name<b.name?-1:1);
  const files=[...n.files].sort((a,b)=>a.name<b.name?-1:1);
  return dirs.concat(files);
}

let active = DATA.findIndex(d=>!d.missing); if(active<0) active=0;
const trees = DATA.map(d=> d.missing? null : buildTree(d.files));
const expanded = DATA.map(()=> new Set());
let filter='';

function defaultExpand(i){
  const e=expanded[i]; e.clear();
  const t=trees[i]; if(!t) return;
  for(const c of t.kids.values()) e.add(c.path);   // top-level packages open
}
DATA.forEach((d,i)=>{ if(!d.missing) defaultExpand(i); });

function matches(n){ return !filter || (n.dir? subtreeHas(n) : n.path.toLowerCase().includes(filter)); }
function subtreeHas(n){
  if(!n.dir) return n.path.toLowerCase().includes(filter);
  for(const c of n.kids.values()) if(subtreeHas(c)) return true;
  for(const f of n.files) if(f.path.toLowerCase().includes(filter)) return true;
  return false;
}

function renderCards(){
  document.getElementById('cards').innerHTML = DATA.map(d=>{
    if(d.missing) return '<div class="card"><h2>'+d.project+'</h2><div class="meta">no lcov.info — run <code>cover/'+d.project+'/script/generate.sh</code></div></div>';
    const t=d.totals, lp=pct(t.lh,t.lf), fp=pct(t.fnh,t.fnf);
    return '<div class="card"><h2>'+d.project+'</h2>'+
      '<div class="big" style="color:'+col(lp)+'">'+lp.toFixed(1)+'%</div>'+
      '<div class="meta">'+t.files+' files · '+t.lf.toLocaleString()+' lines · '+t.fnf.toLocaleString()+' fns</div>'+
      '<div class="metric"><span>lines</span><span>'+t.lh+'/'+t.lf+'</span></div>'+bar(lp)+
      '<div class="metric"><span>functions</span><span>'+t.fnh+'/'+t.fnf+'</span></div>'+bar(fp)+
      '<div class="meta" style="margin-top:9px">'+(d.mtime||'')+'</div></div>';
  }).join('');
}
function renderTabs(){
  document.getElementById('tabs').innerHTML = DATA.map((d,i)=>
    '<div class="tab'+(i===active?' active':'')+'" data-i="'+i+'">'+d.project+
    (d.missing?'':' <span style="opacity:.6">'+d.totals.files+'</span>')+'</div>').join('');
  document.querySelectorAll('.tab').forEach(t=>t.onclick=()=>{active=+t.dataset.i;renderTabs();render();});
}
function row(n,depth){
  const e=expanded[active];
  const kids = n.dir ? kidsOf(n) : [];
  const has = n.dir && kids.length>0;
  const open = has && e.has(n.path);
  const p = pct(n.lh,n.lf);
  const nm = n.dir
    ? n.name.split('/').map((s,i,a)=> i<a.length-1? '<span class="seg">'+s+'/</span>':s).join('')
    : n.name;
  const badge = n.crate? ' <span class="badge">crate</span>':'';
  let h = '<div class="r '+(n.dir?'dir':'file')+'" style="padding-left:'+(14+depth*15)+'px" data-p="'+encodeURIComponent(n.path)+'" data-has="'+(has?1:0)+'">'+
    '<span class="tw">'+(has?(open?'▾':'▸'):'')+'</span>'+
    '<span class="nm" title="'+n.path+'">'+nm+badge+'</span>'+
    '<span class="cnt">'+n.lh+'/'+n.lf+'</span>'+
    '<span class="pctw">'+bar(p)+'<b style="color:'+col(p)+'">'+p.toFixed(0)+'%</b></span>'+
  '</div>';
  if(open){
    for(const c of kids){ if(matches(c)) h += row(c, depth+1); }
  }
  return h;
}
function render(){
  const rows=document.getElementById('rows'), t=trees[active], d=DATA[active];
  if(!t){ rows.innerHTML='<div class="empty">No report for <b>'+d.project+'</b> yet.</div>'; document.getElementById('foot').textContent=''; return; }
  if(filter){ // auto-expand everything matching so hits are visible
    const e=expanded[active];
    (function open(n){ if(n.dir){ if(subtreeHas(n)) e.add(n.path); for(const c of n.kids.values()) open(c);} })(t);
  }
  let h=''; for(const c of kidsOf(t)){ if(matches(c)) h+=row(c,0); }
  rows.innerHTML = h || '<div class="empty">no files match "'+filter+'"</div>';
  document.getElementById('foot').textContent = d.totals.files+' files · '+d.totals.lh+'/'+d.totals.lf+' lines ('+pct(d.totals.lh,d.totals.lf).toFixed(1)+'%)';
  rows.querySelectorAll('.r').forEach(r=>{
    r.onclick=ev=>{
      if(r.dataset.has!=='1') return;
      const p=decodeURIComponent(r.dataset.p), e=expanded[active];
      if(e.has(p)) e.delete(p); else e.add(p);
      render();
    };
  });
}
function eachDir(n,fn){ if(n.dir){ if(n.path) fn(n.path); for(const c of n.kids.values()) eachDir(c,fn);} }
document.getElementById('expand').onclick=()=>{ const e=expanded[active]; e.clear(); if(trees[active]) eachDir(trees[active],p=>e.add(p)); render(); };
document.getElementById('collapse').onclick=()=>{ expanded[active].clear(); render(); };
document.getElementById('filter').oninput=ev=>{ filter=ev.target.value.toLowerCase().trim(); render(); };
document.getElementById('theme').onclick=()=>{ const r=document.documentElement; const cur=r.getAttribute('data-theme')||(matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light'); r.setAttribute('data-theme',cur==='dark'?'light':'dark'); };
document.getElementById('gen').textContent='generated '+${JSON.stringify(new Date().toISOString().replace('T', ' ').slice(0, 19))};
renderCards(); renderTabs(); render();
</script>
</body>
</html>`;

fs.writeFileSync(OUT, html);
const tot = data.filter(d => !d.missing).reduce((a, d) => a + d.totals.files, 0);
console.error(`cover: wrote ${path.relative(REPO_ROOT, OUT)} (${data.length} projects, ${tot} files)`);
