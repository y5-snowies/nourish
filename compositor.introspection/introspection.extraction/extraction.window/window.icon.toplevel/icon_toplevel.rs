//! Read a live toplevel's `xdg_toplevel_icon_v1` icon — the name it declared
//! and/or the pixel buffers it attached.
//!
//! This is the SYNCHRONOUS half of introspection: it needs the wayland surface,
//! so it runs on the compositor thread and its result is handed to hint
//! inference as extra context. Nothing here reaches [`Meta`] or the sampler
//! thread, which walk `/proc` and never see a surface.
//!
//! Buffers are copied out of the client's shm pool immediately (the pointer is
//! only valid for the duration of the map) and converted to the straight
//! `RGBA8888` a renderer wants: wayland's `ARGB8888` is a little-endian 32-bit
//! word — B,G,R,A in memory order — with PREMULTIPLIED alpha.
//!
//! X11 answers the same question through `_NET_WM_ICON`, and [`read`] takes that
//! route for an X11 window. Same output, different packing: EWMH gives CARD32 words
//! with A in the HIGH byte (so the shift is arithmetic, not a memory-order read) and,
//! unlike wayland shm, alpha that is not premultiplied.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::wayland::compositor::with_states;
use smithay::backend::renderer::buffer_dimensions;
use smithay::wayland::shm::with_buffer_contents;
use smithay::wayland::xdg_toplevel_icon::ToplevelIconCachedState;
use std::sync::Arc;
use compositor_introspection_extraction_window_hints_values::values::{IconPixels, ToplevelIcon};

/// Icon edge (px) the compositor would like. A client may attach several sizes;
/// the smallest one at least this big wins, else the biggest on offer.
const PREFERRED_EDGE: i32 = 64;

/// Largest `_NET_WM_ICON` image worth decoding, per edge.
///
/// Generous on purpose: y5's canvas zooms, so an icon with more detail than the
/// current on-screen size is not wasted the way it would be on a fixed desktop, and
/// 1024 is the largest size applications realistically ship. The cap is here to bound
/// the RGBA copy a hostile or careless property could ask for (1024² is already 4 MiB),
/// not to express a preference — `PREFERRED_EDGE` does that.
///
/// Oversized images are SKIPPED, not fatal: the walk carries on and a smaller image
/// from the same property is still used. The read itself is bounded separately, by
/// `X11Surface::icon`.
const MAX_EDGE: u32 = 1024;

/// The decoded `_NET_WM_ICON` of an X11 window, resolved once — see [`read`].
struct X11IconCache(std::sync::OnceLock<Option<ToplevelIcon>>);

/// The icon the client committed for this window, or `None` if it set none.
///
/// Reads the surface's committed (double-buffered) state, so it reflects the
/// last `set_icon` the client actually committed — never a pending one.
pub fn read(window: &Window) -> Option<ToplevelIcon> {
    // X11 carries the icon on the window, not on a role, and it has no name half —
    // `_NET_WM_ICON` is pixels only. A window that sets none still resolves an icon
    // through the desktop entry its `WM_CLASS` names, which is the path every iconless
    // window takes; this is what rescues the ones with no `.desktop` file at all
    // (anything Proton-launched, most notably).
    //
    // CACHED on the window after the first read. `X11Surface::icon` is a blocking
    // `GetProperty` round trip on the window manager's X connection, and this is read per
    // hovered frame in the overview and per sampler batch — a round trip at frame rate for
    // a property that does not change: applications set `_NET_WM_ICON` before mapping, and
    // smithay's `WmWindowProperty` does not report it, so there is nothing to invalidate
    // on. The cell holds the DECODED answer, `None` included.
    if window.is_x11() {
        let data = window.user_data();
        data.insert_if_missing_threadsafe(|| X11IconCache(std::sync::OnceLock::new()));
        let cache = data.get::<X11IconCache>().unwrap();
        return cache
            .0
            .get_or_init(|| {
                let words = compositor_support_smithay_state_window_ident::ident::x11_icon(window);
                let icon = ToplevelIcon { name: None, pixels: words.and_then(|w| decode_net_wm_icon(&w)) };
                (!icon.is_empty()).then_some(icon)
            })
            .clone();
    }
    // `xdg_surface`, not `surface`: `xdg_toplevel_icon_v1` is a wayland protocol and
    // the X11 half is handled above.
    let surface = compositor_support_smithay_state_window_ident::ident::xdg_surface(window)?;
    let (name, buffer) = with_states(&surface, |states| {
        let mut cached = states.cached_state.get::<ToplevelIconCachedState>();
        let current = cached.current();
        (current.icon_name().map(str::to_owned), best_buffer(current.buffers()))
    });
    let icon = ToplevelIcon { name, pixels: buffer.and_then(|b| decode(&b)) };
    (!icon.is_empty()).then_some(icon)
}

/// Pick the buffer to decode: the smallest at least [`PREFERRED_EDGE`] across,
/// else the largest available. Icons are square (the protocol enforces it), so
/// width alone orders them. The buffer's `scale` is not applied — a 64px buffer
/// at scale 2 still has 64px of detail to draw with.
fn best_buffer(buffers: &[(WlBuffer, i32)]) -> Option<WlBuffer> {
    // Geometry without mapping the pool — only the winner is ever read.
    let edge = |b: &WlBuffer| buffer_dimensions(b).map_or(0, |size| size.w);
    let mut atleast: Option<(i32, WlBuffer)> = None;
    let mut largest: Option<(i32, WlBuffer)> = None;
    for (buffer, _scale) in buffers {
        let w = edge(buffer);
        if w <= 0 {
            continue;
        }
        if w >= PREFERRED_EDGE && atleast.as_ref().is_none_or(|(best, _)| w < *best) {
            atleast = Some((w, buffer.clone()));
        }
        if largest.as_ref().is_none_or(|(best, _)| w > *best) {
            largest = Some((w, buffer.clone()));
        }
    }
    atleast.or(largest).map(|(_, buffer)| buffer)
}

/// Copy `buffer` out of the client's pool as straight RGBA. `None` for a format
/// we don't read (shm always offers at least the two below) or a pool too small
/// for the geometry it claims.
fn decode(buffer: &WlBuffer) -> Option<IconPixels> {
    with_buffer_contents(buffer, |pointer, len, data| {
        // Alpha comes from the fourcc table, not a local match: "does this format
        // carry alpha" is a property of the format and had four copies in this
        // tree, one of which had already gone stale.
        let alpha = match smithay::wayland::shm::shm_format_to_fourcc(data.format) {
            Some(code) => !compositor_kernel_vulkan_format_query_base::query::opaque(code),
            None => {
                warn!("toplevel icon: unread shm format {:?}", data.format);
                return None;
            }
        };
        let (width, height, stride) = (data.width, data.height, data.stride);
        if width <= 0 || height <= 0 || stride < width * 4 {
            return None;
        }
        let end = data.offset as isize + (height - 1) as isize * stride as isize + width as isize * 4;
        if data.offset < 0 || end > len as isize {
            warn!("toplevel icon: buffer claims {width}x{height} past the end of its pool");
            return None;
        }

        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            // SAFETY: the row lies inside the mapping — checked above — and the
            // pool stays mapped for the duration of this closure.
            let row = unsafe {
                std::slice::from_raw_parts(
                    pointer.offset(data.offset as isize + y as isize * stride as isize),
                    (width * 4) as usize,
                )
            };
            for pixel in row.chunks_exact(4) {
                // Memory order of a little-endian ARGB8888 word.
                let (b, g, r) = (pixel[0], pixel[1], pixel[2]);
                let a = if alpha { pixel[3] } else { 255 };
                let (r, g, b) = if alpha { unpremultiply(r, g, b, a) } else { (r, g, b) };
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
        Some(IconPixels { width: width as u32, height: height as u32, rgba: Arc::new(rgba) })
    })
    .ok()
    .flatten()
}

/// wayland shm alpha is premultiplied; renderers taking raw RGBA expect it
/// straight. Fully transparent pixels carry no colour to recover.
fn unpremultiply(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    if a == 0 || a == 255 {
        return (r, g, b);
    }
    let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
    (un(r), un(g), un(b))
}

/// Decode `_NET_WM_ICON` into the best-sized image it offers.
///
/// EWMH packs images end to end: `width, height, width*height` pixels, repeated. Each
/// pixel is a CARD32 with A in the high byte down to B in the low one — an arithmetic
/// layout, NOT a memory-order one, so it is shifted rather than indexed and the
/// machine's endianness does not enter into it.
///
/// Alpha is taken as straight. EWMH does not say either way, and the implementations
/// that matter write what GdkPixbuf and Qt hold, which is unpremultiplied; treating it
/// as premultiplied would visibly brighten every semi-transparent edge.
///
/// A truncated or absurd header ends the walk rather than failing the whole read: the
/// images already collected are still good, and the property is client-controlled
/// input that must not be able to panic the compositor.
fn decode_net_wm_icon(words: &[u32]) -> Option<IconPixels> {
    let mut best: Option<(u32, IconPixels)> = None;
    let mut i = 0usize;
    while i + 2 <= words.len() {
        let (width, height) = (words[i], words[i + 1]);
        let Some(count) = width.checked_mul(height).map(|n| n as usize) else { break };
        if width == 0 || height == 0 || i + 2 + count > words.len() {
            break;
        }
        let pixels = &words[i + 2..i + 2 + count];
        i += 2 + count;

        // Skip rather than stop: an outsized image says nothing about the ones after
        // it, and a property is free to list 1024 before 48.
        if width > MAX_EDGE || height > MAX_EDGE {
            continue;
        }

        // Keep the smallest image at least PREFERRED_EDGE across, else the largest —
        // the same rule the wayland half applies, so a window's icon does not change
        // size depending on which protocol it arrived by.
        let better = match best.as_ref() {
            None => true,
            Some((edge, _)) => match (*edge >= PREFERRED_EDGE as u32, width >= PREFERRED_EDGE as u32) {
                (false, true) => true,
                (true, true) => width < *edge,
                (false, false) => width > *edge,
                (true, false) => false,
            },
        };
        if !better {
            continue;
        }
        let mut rgba = Vec::with_capacity(count * 4);
        for word in pixels {
            rgba.extend_from_slice(&[
                (word >> 16) as u8,
                (word >> 8) as u8,
                *word as u8,
                (word >> 24) as u8,
            ]);
        }
        best = Some((width, IconPixels { width, height, rgba: Arc::new(rgba) }));
    }
    best.map(|(_, pixels)| pixels)
}
