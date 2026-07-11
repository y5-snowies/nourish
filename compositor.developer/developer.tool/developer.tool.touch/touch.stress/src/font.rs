//! A self-contained 5x7 bitmap font (packed into 8x8 cells), enough to render button
//! labels, the command log, and the subject's state overlay. No external font asset.
//!
//! Each glyph is `[u8; 8]`, one byte per row (top first). Within a row, the 5 used columns
//! are the high bits: bit 7 = leftmost column ... bit 3 = 5th column. So a row written
//! `0b01110_000` reads as the pixels ` ### `.

use crate::canvas::Canvas;

const W: i32 = 5; // glyph width in pixels
const ADV: i32 = 6; // per-character advance (glyph + 1px gap)

/// Pixel width of `s` rendered at `scale`.
pub fn text_width(s: &str, scale: i32) -> i32 {
    s.chars().count() as i32 * ADV * scale
}

/// Line height at `scale`.
pub fn line_height(scale: i32) -> i32 {
    8 * scale
}

/// Blit `s` at top-left (x, y) in `argb`, each font pixel drawn as a `scale` x `scale` block.
pub fn text(c: &mut Canvas, x: i32, y: i32, scale: i32, argb: u32, s: &str) {
    let mut cx = x;
    for ch in s.chars() {
        let g = glyph(ch);
        for (ry, row) in g.iter().enumerate() {
            for col in 0..W {
                if (row >> (7 - col)) & 1 == 1 {
                    c.rect(cx + col * scale, y + ry as i32 * scale, scale, scale, argb);
                }
            }
        }
        cx += ADV * scale;
    }
}

/// Look up a glyph, upper-casing letters and falling back to a blank for the unknown.
fn glyph(ch: char) -> [u8; 8] {
    let c = ch.to_ascii_uppercase();
    match c {
        ' ' => BLANK,
        'A' => A, 'B' => B, 'C' => C, 'D' => D, 'E' => E, 'F' => F, 'G' => G,
        'H' => H, 'I' => I, 'J' => J, 'K' => K, 'L' => L, 'M' => M, 'N' => N,
        'O' => O, 'P' => P, 'Q' => Q, 'R' => R, 'S' => S, 'T' => T, 'U' => U,
        'V' => V, 'W' => WW, 'X' => X, 'Y' => Y, 'Z' => Z,
        '0' => D0, '1' => D1, '2' => D2, '3' => D3, '4' => D4,
        '5' => D5, '6' => D6, '7' => D7, '8' => D8, '9' => D9,
        ':' => COLON, '+' => PLUS, '-' => MINUS, '=' => EQ, ',' => COMMA,
        '.' => DOT, '/' => SLASH, '(' => LPAREN, ')' => RPAREN, '%' => PCT,
        '>' => GT, '<' => LT, '#' => HASH, '!' => BANG, '?' => QUEST,
        '*' => STAR, '_' => UNDER,
        _ => UNKNOWN,
    }
}

#[rustfmt::skip]
const BLANK: [u8; 8] = [0,0,0,0,0,0,0,0];
#[rustfmt::skip]
const UNKNOWN: [u8; 8] = [0b11111000,0b10001000,0b10001000,0b10001000,0b10001000,0b10001000,0b11111000,0];

#[rustfmt::skip] const A: [u8;8] = [0b01110000,0b10001000,0b10001000,0b11111000,0b10001000,0b10001000,0b10001000,0];
#[rustfmt::skip] const B: [u8;8] = [0b11110000,0b10001000,0b10001000,0b11110000,0b10001000,0b10001000,0b11110000,0];
#[rustfmt::skip] const C: [u8;8] = [0b01110000,0b10001000,0b10000000,0b10000000,0b10000000,0b10001000,0b01110000,0];
#[rustfmt::skip] const D: [u8;8] = [0b11110000,0b10001000,0b10001000,0b10001000,0b10001000,0b10001000,0b11110000,0];
#[rustfmt::skip] const E: [u8;8] = [0b11111000,0b10000000,0b10000000,0b11110000,0b10000000,0b10000000,0b11111000,0];
#[rustfmt::skip] const F: [u8;8] = [0b11111000,0b10000000,0b10000000,0b11110000,0b10000000,0b10000000,0b10000000,0];
#[rustfmt::skip] const G: [u8;8] = [0b01110000,0b10001000,0b10000000,0b10111000,0b10001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const H: [u8;8] = [0b10001000,0b10001000,0b10001000,0b11111000,0b10001000,0b10001000,0b10001000,0];
#[rustfmt::skip] const I: [u8;8] = [0b11111000,0b00100000,0b00100000,0b00100000,0b00100000,0b00100000,0b11111000,0];
#[rustfmt::skip] const J: [u8;8] = [0b00111000,0b00010000,0b00010000,0b00010000,0b00010000,0b10010000,0b01100000,0];
#[rustfmt::skip] const K: [u8;8] = [0b10001000,0b10010000,0b10100000,0b11000000,0b10100000,0b10010000,0b10001000,0];
#[rustfmt::skip] const L: [u8;8] = [0b10000000,0b10000000,0b10000000,0b10000000,0b10000000,0b10000000,0b11111000,0];
#[rustfmt::skip] const M: [u8;8] = [0b10001000,0b11011000,0b10101000,0b10101000,0b10001000,0b10001000,0b10001000,0];
#[rustfmt::skip] const N: [u8;8] = [0b10001000,0b10001000,0b11001000,0b10101000,0b10011000,0b10001000,0b10001000,0];
#[rustfmt::skip] const O: [u8;8] = [0b01110000,0b10001000,0b10001000,0b10001000,0b10001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const P: [u8;8] = [0b11110000,0b10001000,0b10001000,0b11110000,0b10000000,0b10000000,0b10000000,0];
#[rustfmt::skip] const Q: [u8;8] = [0b01110000,0b10001000,0b10001000,0b10001000,0b10101000,0b10010000,0b01101000,0];
#[rustfmt::skip] const R: [u8;8] = [0b11110000,0b10001000,0b10001000,0b11110000,0b10100000,0b10010000,0b10001000,0];
#[rustfmt::skip] const S: [u8;8] = [0b01111000,0b10000000,0b10000000,0b01110000,0b00001000,0b00001000,0b11110000,0];
#[rustfmt::skip] const T: [u8;8] = [0b11111000,0b00100000,0b00100000,0b00100000,0b00100000,0b00100000,0b00100000,0];
#[rustfmt::skip] const U: [u8;8] = [0b10001000,0b10001000,0b10001000,0b10001000,0b10001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const V: [u8;8] = [0b10001000,0b10001000,0b10001000,0b10001000,0b10001000,0b01010000,0b00100000,0];
#[rustfmt::skip] const WW:[u8;8] = [0b10001000,0b10001000,0b10001000,0b10101000,0b10101000,0b11011000,0b10001000,0];
#[rustfmt::skip] const X: [u8;8] = [0b10001000,0b10001000,0b01010000,0b00100000,0b01010000,0b10001000,0b10001000,0];
#[rustfmt::skip] const Y: [u8;8] = [0b10001000,0b10001000,0b01010000,0b00100000,0b00100000,0b00100000,0b00100000,0];
#[rustfmt::skip] const Z: [u8;8] = [0b11111000,0b00001000,0b00010000,0b00100000,0b01000000,0b10000000,0b11111000,0];

#[rustfmt::skip] const D0:[u8;8] = [0b01110000,0b10001000,0b10011000,0b10101000,0b11001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const D1:[u8;8] = [0b00100000,0b01100000,0b00100000,0b00100000,0b00100000,0b00100000,0b01110000,0];
#[rustfmt::skip] const D2:[u8;8] = [0b01110000,0b10001000,0b00001000,0b00010000,0b00100000,0b01000000,0b11111000,0];
#[rustfmt::skip] const D3:[u8;8] = [0b11111000,0b00010000,0b00100000,0b00010000,0b00001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const D4:[u8;8] = [0b00010000,0b00110000,0b01010000,0b10010000,0b11111000,0b00010000,0b00010000,0];
#[rustfmt::skip] const D5:[u8;8] = [0b11111000,0b10000000,0b11110000,0b00001000,0b00001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const D6:[u8;8] = [0b00110000,0b01000000,0b10000000,0b11110000,0b10001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const D7:[u8;8] = [0b11111000,0b00001000,0b00010000,0b00100000,0b01000000,0b01000000,0b01000000,0];
#[rustfmt::skip] const D8:[u8;8] = [0b01110000,0b10001000,0b10001000,0b01110000,0b10001000,0b10001000,0b01110000,0];
#[rustfmt::skip] const D9:[u8;8] = [0b01110000,0b10001000,0b10001000,0b01111000,0b00001000,0b00010000,0b01100000,0];

#[rustfmt::skip] const COLON:[u8;8] = [0,0b00100000,0b00100000,0,0b00100000,0b00100000,0,0];
#[rustfmt::skip] const PLUS: [u8;8] = [0,0b00100000,0b00100000,0b11111000,0b00100000,0b00100000,0,0];
#[rustfmt::skip] const MINUS:[u8;8] = [0,0,0,0b11111000,0,0,0,0];
#[rustfmt::skip] const EQ:   [u8;8] = [0,0,0b11111000,0,0b11111000,0,0,0];
#[rustfmt::skip] const COMMA:[u8;8] = [0,0,0,0,0,0b00100000,0b00100000,0b01000000];
#[rustfmt::skip] const DOT:  [u8;8] = [0,0,0,0,0,0b00100000,0b00100000,0];
#[rustfmt::skip] const SLASH:[u8;8] = [0b00001000,0b00001000,0b00010000,0b00100000,0b01000000,0b10000000,0b10000000,0];
#[rustfmt::skip] const LPAREN:[u8;8]= [0b00010000,0b00100000,0b01000000,0b01000000,0b01000000,0b00100000,0b00010000,0];
#[rustfmt::skip] const RPAREN:[u8;8]= [0b01000000,0b00100000,0b00010000,0b00010000,0b00010000,0b00100000,0b01000000,0];
#[rustfmt::skip] const PCT:  [u8;8] = [0b11001000,0b11010000,0b00100000,0b01011000,0b10011000,0,0,0];
#[rustfmt::skip] const GT:   [u8;8] = [0b10000000,0b01000000,0b00100000,0b01000000,0b10000000,0,0,0];
#[rustfmt::skip] const LT:   [u8;8] = [0b00010000,0b00100000,0b01000000,0b00100000,0b00010000,0,0,0];
#[rustfmt::skip] const HASH: [u8;8] = [0b01010000,0b01010000,0b11111000,0b01010000,0b11111000,0b01010000,0b01010000,0];
#[rustfmt::skip] const BANG: [u8;8] = [0b00100000,0b00100000,0b00100000,0b00100000,0b00100000,0,0b00100000,0];
#[rustfmt::skip] const QUEST:[u8;8] = [0b01110000,0b10001000,0b00010000,0b00100000,0b00100000,0,0b00100000,0];
#[rustfmt::skip] const STAR: [u8;8] = [0,0b00100000,0b10101000,0b01110000,0b10101000,0b00100000,0,0];
#[rustfmt::skip] const UNDER:[u8;8] = [0,0,0,0,0,0,0,0b11111000];
