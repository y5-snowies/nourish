// JSONC for the workspace tooling: JSON with `//` and `/* */` comments.
//
// `crate.json` and the two root catalogs are JSONC because the per-dependency
// rationale that used to live as comments in Cargo.toml has to live somewhere,
// and losing it was the one real cost of moving to generated manifests.
//
// No npm dependency, deliberately — every script in this repo runs on a bare
// `node` with `fs` and `path` and nothing else.
//
// The stripper is STRING-AWARE. A regex that just deletes `//...` to end of line
// corrupts any value containing a URL:
//     "repository": "https://github.com/..."   ->   "repository": "https:
// so the scanner tracks whether it is inside a string literal (and whether the
// previous character was an escape) before it treats `/` as a comment opener.
//
// Comments are replaced with spaces rather than removed, so byte offsets are
// preserved — that is what lets `commentsFor` map a comment back to the array
// entry it belongs to without re-parsing.

'use strict';

const fs = require('fs');

/// Replace every comment in `text` with spaces of equal length, preserving all
/// offsets, newlines and string contents. Also tolerates a trailing comma before
/// `}` or `]`, which hand-edited JSONC accumulates.
function strip(text) {
    const out = Array.from(text);
    let inString = false;
    let escaped = false;
    let i = 0;

    while (i < text.length) {
        const c = text[i];

        if (inString) {
            if (escaped) escaped = false;
            else if (c === '\\') escaped = true;
            else if (c === '"') inString = false;
            i++;
            continue;
        }

        if (c === '"') {
            inString = true;
            i++;
            continue;
        }

        if (c === '/' && text[i + 1] === '/') {
            while (i < text.length && text[i] !== '\n') out[i++] = ' ';
            continue;
        }

        if (c === '/' && text[i + 1] === '*') {
            const end = text.indexOf('*/', i + 2);
            const stop = end === -1 ? text.length : end + 2;
            // Keep newlines so line numbers in parse errors stay true.
            for (; i < stop; i++) if (out[i] !== '\n') out[i] = ' ';
            continue;
        }

        i++;
    }

    // Trailing commas: `,` followed only by whitespace and a closer.
    let s = out.join('');
    s = s.replace(/,(\s*[}\]])/g, ' $1');
    return s;
}

/// Parse JSONC text. `label` only improves the error message.
function parse(text, label) {
    try {
        return JSON.parse(strip(text));
    } catch (e) {
        throw new Error(`${label || 'jsonc'}: ${e.message}`);
    }
}

/// Read and parse a JSONC file.
function read(file) {
    return parse(fs.readFileSync(file, 'utf-8'), file);
}

/// The comment lines attached to each entry of a top-level string array.
///
/// Returns `{ <array entry>: ["comment line", ...] }` for the array at
/// `key`, mapping each string element to the `//` comment lines directly above
/// it. This is what lets the generator re-emit a dependency's rationale above its
/// line in Cargo.toml, so a generated manifest reads like the hand-written one it
/// replaced.
///
/// Deliberately line-based rather than a real CST: the shape it has to handle is
/// only ever the `deps`/`dev`/`build` arrays of a `crate.json`, one string per
/// line, which the extractor writes and which nothing else edits by machine.
function commentsFor(text, key) {
    const lines = text.split('\n');
    const attached = {};
    let depth = 0;
    let inArray = false;
    let pending = [];

    for (const raw of lines) {
        const line = raw.trim();
        if (!inArray) {
            // `"key": [` — start collecting.
            if (new RegExp(`^"${key}"\\s*:\\s*\\[`).test(line)) {
                inArray = true;
                depth = 0;
                pending = [];
            }
            continue;
        }
        if (line.startsWith(']')) break;
        if (line.startsWith('//')) {
            pending.push(line.replace(/^\/\/\s?/, ''));
            continue;
        }
        const m = line.match(/^"([^"]+)"/);
        if (m) {
            if (pending.length) attached[m[1]] = pending;
            pending = [];
            continue;
        }
        if (line === '') continue;
        // Anything else (nested structure) — give up rather than guess.
        pending = [];
        void depth;
    }
    return attached;
}

module.exports = { strip, parse, read, commentsFor };
