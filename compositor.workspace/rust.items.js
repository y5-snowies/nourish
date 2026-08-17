// Split Rust source into TOP-LEVEL items, comment/string aware.
//
// Exists for the `shell` lint rule (workspace.lint.js): lib.rs and mod.rs must
// declare structure and re-export it, never define it, and answering "is this
// file only `mod`/`use`?" by regex over lines is wrong in both directions — a
// `//` inside a string literal, a brace inside a char literal, or a `use` in the
// middle of a function body all break it.
//
// NOT a Rust parser. It tracks nesting and literal context well enough to find
// where one top-level item ends and the next begins, then classifies each by its
// leading keyword. That is the whole job.
//
// No npm dependencies, same as jsonc.js.

// Advance past a string/char literal starting at `i` (which indexes the opening
// delimiter). Returns the index just past the closing delimiter.
//
// Raw strings (r"…", r#"…"#, br##"…"##) close only on a quote followed by the
// same number of hashes, so the hash count has to be carried.
function skipString(src, i, quote, hashes) {
  i += 1;
  while (i < src.length) {
    const c = src[i];
    if (hashes === 0 && c === '\\') { i += 2; continue; } // escapes only in cooked literals
    if (c === quote) {
      if (hashes === 0) return i + 1;
      let n = 0;
      while (src[i + 1 + n] === '#' && n < hashes) n += 1;
      if (n === hashes) return i + 1 + n;
    }
    i += 1;
  }
  return i;
}

// A `'` opens a char literal only when it is not a lifetime/label. `'a`, `'_`,
// `'static` are lifetimes; `'a'`, `'\n'`, `'}'` are chars. The discriminator is
// the closing quote: a char literal is at most one escape sequence long.
function isCharLiteral(src, i) {
  if (src[i + 1] === '\\') return true;               // '\n' '\'' '\\'
  if (src[i + 2] === "'") return true;                // 'a'
  return false;                                       // 'lifetime
}

// Extend an item's end past a `//` comment sitting on the SAME line as its
// terminator — `pub const FOV: f32 = 0.69; // ~40°` is one item, and treating the
// comment as the start of the NEXT item detaches it from what it describes.
function trailingComment(src, end) {
  let j = end;
  while (src[j] === ' ' || src[j] === '\t') j += 1;
  if (src[j] !== '/' || src[j + 1] !== '/') return end;
  while (j < src.length && src[j] !== '\n') j += 1;
  return j;
}

// Index of the next character that is neither whitespace nor comment, or -1.
function nextSignificant(src, i) {
  while (i < src.length) {
    if (/\s/.test(src[i])) { i += 1; continue; }
    if (src[i] === '/' && src[i + 1] === '/') {
      while (i < src.length && src[i] !== '\n') i += 1;
      continue;
    }
    if (src[i] === '/' && src[i + 1] === '*') {
      let nest = 1; i += 2;
      while (i < src.length && nest > 0) {
        if (src[i] === '/' && src[i + 1] === '*') { nest += 1; i += 1; }
        else if (src[i] === '*' && src[i + 1] === '/') { nest -= 1; i += 1; }
        i += 1;
      }
      continue;
    }
    return i;
  }
  return -1;
}

/// Top-level items of `src`, in order.
///
/// Each item is `{ text, kind, line }` where `line` is 1-indexed and `kind` is
/// the classifier below. Attributes and doc comments attach to the item that
/// follows them, so `#[derive(Clone)] pub struct Foo {}` is ONE item of kind
/// `struct` — which is what makes the lint's message point at the right thing.
function topLevelItems(src) {
  const items = [];
  let depth = 0;
  let start = null;      // index where the pending item begins (its first comment/attr)
  let line = 1;
  let startLine = 1;
  let i = 0;

  const begin = () => { if (start === null && depth === 0) { start = i; startLine = line; } };

  const flush = (end) => {
    if (start === null) return;
    const raw = src.slice(start, end);
    const text = raw.trim();
    // `end` is exclusive and `raw` may carry trailing whitespace; report the span
    // of the trimmed text so a caller can splice on it.
    if (text) items.push({
      text,
      kind: classify(text),
      line: startLine,
      start: start + (raw.length - raw.trimStart().length),
      end: end - (raw.length - raw.trimEnd().length),
    });
    start = null;
  };

  while (i < src.length) {
    const c = src[i];

    if (c === '\n') { line += 1; i += 1; continue; }

    // A comment BEGINS the item that follows it — a doc comment or an
    // explanatory block above a struct belongs to that struct, and a migration
    // that moves the struct has to carry it along.
    if (c === '/' && src[i + 1] === '/') {
      begin();
      while (i < src.length && src[i] !== '\n') i += 1;
      continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      begin();
      let nest = 1; i += 2;
      while (i < src.length && nest > 0) {
        if (src[i] === '\n') line += 1;
        else if (src[i] === '/' && src[i + 1] === '*') { nest += 1; i += 1; }
        else if (src[i] === '*' && src[i + 1] === '/') { nest -= 1; i += 1; }
        i += 1;
      }
      continue;
    }

    if (/\s/.test(c)) { i += 1; continue; }

    begin();

    // Raw / byte-string prefixes, then the literal itself.
    if (c === 'r' || c === 'b') {
      let j = i + 1;
      if (src[i] === 'b' && src[j] === 'r') j += 1;
      let hashes = 0;
      while (src[j + hashes] === '#') hashes += 1;
      if (src[j + hashes] === '"') {
        i = skipString(src, j + hashes, '"', hashes);
        continue;
      }
    }
    if (c === '"') { i = skipString(src, i, '"', 0); continue; }
    if (c === "'" && isCharLiteral(src, i)) { i = skipString(src, i, "'", 0); continue; }

    if (c === '{' || c === '(' || c === '[') { depth += 1; i += 1; continue; }
    if (c === '}' || c === ')' || c === ']') {
      depth -= 1;
      i += 1;
      // A brace closing back to depth 0 ends a braced item (fn/impl/mod-with-body)
      // — UNLESS a `;` follows, in which case the braces were interior syntax and
      // the `;` is the real terminator: `use a::{b, c};`, `static X: T = S { … };`.
      // Splitting there would leave a bare `;` masquerading as its own item.
      if (depth === 0 && c === '}') {
        const semi = nextSignificant(src, i);
        if (semi !== -1 && src[semi] === ';') {
          line += (src.slice(i, semi).match(/\n/g) || []).length;
          i = semi + 1;
        }
        i = trailingComment(src, i);   // consume it, or it re-opens as its own item
        flush(i);
      }
      continue;
    }
    // A `;` at depth 0 ends a statement-like item (use, mod decl, type alias,
    // macro invocation with `;`).
    if (c === ';' && depth === 0) { i = trailingComment(src, i + 1); flush(i); continue; }

    i += 1;
  }
  flush(src.length);
  return items;
}

// Classify an item by its leading keyword, after stripping the attributes and
// doc comments that precede it.
function classify(text) {
  let s = text;
  // Peel leading attributes / comments until a keyword is exposed.
  for (;;) {
    const before = s;
    s = s.replace(/^\s*(?:\/\/[^\n]*|\/\*[\s\S]*?\*\/)\s*/, '');
    s = s.replace(/^\s*#!?\[(?:[^\[\]]|\[[^\]]*\])*\]\s*/, '');
    if (s === before) break;
  }
  s = s.replace(/^\s*pub\s*(?:\([^)]*\)\s*)?/, '');   // pub / pub(crate) / pub(in …)
  s = s.replace(/^\s*(?:default\s+|unsafe\s+|async\s+|const\s+(?=fn\b)|extern\s+"[^"]*"\s+)+/, '');

  // Nothing left after peeling: the "item" was only attributes and/or comments.
  // A trailing comment at end of file, or a file-level `//!` block, lands here.
  if (!s.trim()) return /#!\[/.test(text) ? 'inner-attr' : 'comment';

  const kw = /^([A-Za-z_][A-Za-z0-9_]*)/.exec(s);
  if (!kw) return 'other';
  const word = kw[1];

  switch (word) {
    case 'use': return 'use';
    case 'extern': return /^extern\s+crate\b/.test(s) ? 'extern-crate' : 'other';
    // Declaration vs inline body. Test the CODE, not the raw text: a doc comment
    // above the item routinely contains a brace (`SaveClicked { updated_plan }`),
    // and matching that made every such `pub mod x;` look like an inline module.
    case 'mod': return /^mod\s+(?:r#)?[A-Za-z_][A-Za-z0-9_]*\s*;/.test(s) ? 'mod' : 'mod-inline';
    case 'fn': return 'fn';
    case 'struct': return 'struct';
    case 'enum': return 'enum';
    case 'trait': return 'trait';
    case 'impl': return 'impl';
    case 'type': return 'type-alias';
    case 'const': return 'const';
    case 'static': return 'static';
    case 'union': return 'union';
    case 'macro_rules': return 'macro_rules';
    default: break;
  }
  // `instance!(…);` and friends — a bare macro invocation at item position.
  if (/^[A-Za-z_][A-Za-z0-9_:]*\s*!/.test(s)) {
    const name = /^([A-Za-z_][A-Za-z0-9_:]*)\s*!/.exec(s)[1];
    return 'macro:' + name;
  }
  return 'other';
}

// An item with no attributes/comments is "bare" — used to keep an inner
// attribute (`#![allow(...)]`) distinguishable from a decorated item.
function isInnerAttrOnly(text) {
  return /^\s*(?:#!\[(?:[^\[\]]|\[[^\]]*\])*\]\s*)+$/.test(text);
}

// ── The SHELL rule ────────────────────────────────────────────────────────────
// A `lib.rs` / `mod.rs` DECLARES structure and RE-EXPORTS it. It never defines
// anything. That keeps the crate's public shape readable in one screen and makes
// a review of "what does this crate expose" a diff of re-exports rather than a
// reread of the file — which is the whole point of the flat one-module layout.
//
// Permitted at any depth:
//   #![…]                      inner attributes (lint/feature gates)
//   extern crate …;            incl. #[macro_use], which is how the logging
//                              macros reach the crate
//   mod x;  pub mod x;         module declarations
//   use …;  pub use …;         re-exports
//   instance!(…);              the per-crate logging instance declaration
//   mod x { … }                inline module whose body satisfies this same rule
//                              (a re-export grouping, not a definition site)
const SHELL_ALLOWED = new Set(['use', 'extern-crate', 'mod', 'inner-attr', 'macro:instance', 'comment']);

// Index of the first `{` that is real code — not inside a comment or a literal.
function codeBrace(text) {
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === '/' && text[i + 1] === '/') { while (i < text.length && text[i] !== '\n') i += 1; continue; }
    if (c === '/' && text[i + 1] === '*') {
      let nest = 1; i += 2;
      while (i < text.length && nest > 0) {
        if (text[i] === '/' && text[i + 1] === '*') { nest += 1; i += 1; }
        else if (text[i] === '*' && text[i + 1] === '/') { nest -= 1; i += 1; }
        i += 1;
      }
      continue;
    }
    if (c === '"') { i = skipString(text, i, '"', 0); continue; }
    if (c === "'" && isCharLiteral(text, i)) { i = skipString(text, i, "'", 0); continue; }
    if (c === '{') return i;
    i += 1;
  }
  return -1;
}

/// Items in `src` that a shell file may not contain, innermost-first per item.
/// Returns `[{ kind, line, text }]`; empty means the file conforms.
function shellViolations(src) {
  const bad = [];
  const scan = (text, lineBase) => {
    for (const item of topLevelItems(text)) {
      const line = lineBase + item.line - 1;
      if (SHELL_ALLOWED.has(item.kind)) continue;
      if (item.kind === 'mod-inline') {
        // Recurse: an inline mod is a grouping only if everything inside it is.
        // The brace has to be found in code — a doc comment above the item can
        // contain one, and slicing from there would parse prose as Rust.
        const open = codeBrace(item.text);
        const close = item.text.lastIndexOf('}');
        if (open === -1 || close <= open) { bad.push({ kind: item.kind, line, text: item.text }); continue; }
        const head = item.text.slice(0, open);
        scan(item.text.slice(open + 1, close), line + (head.match(/\n/g) || []).length);
        continue;
      }
      bad.push({ kind: item.kind, line, text: item.text });
    }
  };
  scan(src, 1);
  return bad;
}

module.exports = { topLevelItems, classify, isInnerAttrOnly, shellViolations, SHELL_ALLOWED };
