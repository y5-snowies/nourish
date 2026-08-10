//! What the engine knows about one world drawable beyond its geometry, and the
//! ONE function that packs it into the shader's `attrs` lane.
//!
//! # Why a crate of its own
//!
//! The `attrs[i]` lane had two independent producers — the compositor's own
//! composite (`renderer.core`) and the worker's (`two.worker`) — each writing
//! `[kind, alpha, 0, 0]` from its own literal. That was survivable only while the
//! layout was two fields nobody was adding to. The moment a descriptor is added to
//! one and not the other, a bundle works inline and silently does nothing when it
//! offloads, which is the hardest class of bug this feature can produce: the
//! shader is correct, the manifest is correct, and the effect depends on a
//! placement decision the author never made.
//!
//! So the layout is a crate, it depends on nothing, and both producers call
//! [`attrs`]. A new descriptor is one edit here and a compile error at every site
//! that has not been told about it.
//!
//! # The lane
//!
//! ```text
//! attrs.x = kind    0 = client window, 1 = iced-world panel
//! attrs.y = alpha   the drawable's own opacity
//! attrs.z = flags   this module's bits, as an exact integer
//! attrs.w = free    reserved for the stable per-window index
//! ```
//!
//! `.x`/`.y` are load-bearing for every shipped bundle and must not move — see
//! `renderer.graph`'s ABI test, which exists because they did once.
//!
//! `.w` is left free on purpose rather than filled with the next boolean that
//! comes along. Grouping a window's elements — a window with subsurfaces or
//! decorations occupies SEVERAL entries and nothing currently says which — needs
//! an index, not a flag, and it is the one descriptor a future accumulator cannot
//! work without, since accumulation needs a key that survives between frames.
//!
//! # Why bits rather than lanes
//!
//! An f32 represents integers exactly up to 2^24, so one lane carries
//! [`MAX_FLAGS`] independent booleans and a shader reads them with
//! `(u32(attrs.z) & FLAG) != 0u`. Spending a whole lane on each boolean would
//! have exhausted the block after two, and widening the block is not cheap: a
//! fourth `vec4` array puts the UBO past the 16 KiB every Vulkan implementation
//! guarantees.

/// This drawable's window holds keyboard focus.
///
/// Focus, not stacking: a window can be front-most and unfocused (and, under a
/// click-to-focus interaction that has not been clicked yet, focused and not
/// front-most), so [`TOPMOST`] is a separate bit rather than the same one.
pub const ACTIVATED: u32 = 1 << 0;

/// This drawable belongs to the front-most window in the world's draw order.
///
/// Set for every element of that window — its surface tree and its decorations —
/// not only the one that happens to be drawn last.
pub const TOPMOST: u32 = 1 << 1;

/// The window is fullscreen.
pub const FULLSCREEN: u32 = 1 << 2;

/// The window is mid-resize: a gesture is live, or one has ended and the client
/// has not yet committed a buffer at the new size.
///
/// The compositor's own stretch authority, the same one the render fit and the
/// letterbox read, rather than the xdg `Resizing` state — that one is a property
/// of a configure the client may not have acked, so it is both later than the
/// gesture and stale after it.
pub const RESIZING: u32 = 1 << 3;

/// The window is being dragged: an interactive move grab is live and this window
/// is one of the drawables it is carrying.
///
/// Note the asymmetry with [`RESIZING`], which is deliberate rather than an
/// oversight. A resize is not finished when the gesture ends — the client still
/// has to commit a buffer at the new size, and the flag stays up across that wait
/// because the window is visibly unsettled the whole time. A move needs no client
/// roundtrip at all: the compositor maps the element itself, so the gesture
/// ending IS the move ending, and there is nothing to wait out.
///
/// It is the gesture, not the position. A window relocated by the navigator, by
/// tiling or by a world animation is not "moving" here, because nothing about
/// those is a drag the user is holding.
pub const MOVING: u32 = 1 << 4;

/// In the canvas SELECTION — the set a group move/resize carries, and what the
/// select box is drawn around. Distinct from `ACTIVATED`: focus is one window,
/// a selection is many, and a bundle that highlights a group needs the second.
pub const SELECTED: u32 = 1 << 5;

/// The PRIMARY member of a multi-window selection — the one a group operation
/// treats as the reference. Always also [`SELECTED`].
///
/// There is no primary for a selection of one: a single selected window carries
/// `SELECTED` alone. So this is not "the first one picked", it is specifically
/// "the anchor of a group", and a bundle that outlines the group differently from
/// its anchor is the case it exists for.
///
/// FLAG ONLY, with no moment beside it, unlike [`SELECTED`]. Which member is
/// primary changes within a selection that is otherwise unchanged, so a timestamp
/// would answer "when did this become the anchor" — a question no effect has
/// wanted, and one that costs a lane in a fixed-width uniform to hold. Add it
/// when something needs it, not before.
pub const PRIMARY: u32 = 1 << 6;

/// Every bit this version defines, for a consumer validating a value it did not
/// produce. Unknown bits are not an error — a bundle written against an older
/// engine simply tests fewer of them.
pub const ALL: u32 =
    ACTIVATED | TOPMOST | FULLSCREEN | RESIZING | MOVING | SELECTED | PRIMARY;

/// How many flags the lane can hold: an f32 mantissa is exact to 2^24, and past
/// that a bit test starts answering about a rounded number.
pub const MAX_FLAGS: u32 = 24;

/// `bit` when `on`, nothing otherwise — so a caller reads as a list of what is
/// true rather than as a chain of `if`s.
pub const fn when(bit: u32, on: bool) -> u32 {
    if on { bit } else { 0 }
}

/// Pack one drawable's descriptors into the shader's `attrs[i]`.
///
/// THE packer. Both composite paths call it; nothing else may build this array.
pub fn attrs(kind: u8, alpha: f32, flags: u32) -> [f32; 4] {
    debug_assert!(flags >> MAX_FLAGS == 0, "descriptor flags past bit {MAX_FLAGS} are not exact in f32");
    [kind as f32, alpha, flags as f32, 0.0]
}

/// One entry's timestamps: WHEN each event last happened, on the shared clock.
///
/// ```text
/// life.x  = opened            first drawn
/// life.y  = entered           last became on-screen
/// life.z  = left              last became off-screen
/// life.w  = reserved
/// state.x = focused           last took keyboard focus
/// state.y = topmost           last became the front-most window
/// state.z = selected          last joined the canvas selection
/// state.w = deselected        last left it
/// drag.x  = resize started
/// drag.y  = resize ended
/// drag.z  = move started
/// drag.w  = move ended
/// ```
///
/// The two gestures share `drag` rather than resize sitting in `state` next to
/// the focus moments: a pass that reacts to one almost always wants the other,
/// and reading `state.zw` for resize while reading `drag.xy` for move is the kind
/// of split nobody remembers the right way round.
///
/// Absolute, not ages — see `window.clock`. A pass reads
/// `t - times.life[i].x` and gets an age that advances every frame on either
/// path. `clock::NEVER` for an event that has not happened.
///
/// Grouped arrays rather than twelve loose lanes because a WGSL uniform array is
/// `vec4`-strided either way: the grouping costs exactly what a flat list would
/// and reads as what it is.
///
/// # `left` is only ever readable in hindsight
///
/// A window that is off-screen contributes no entry, so nothing can read the
/// moment it left WHILE it is gone — there is no `i` to index. It becomes
/// readable when the window comes back, which is what makes it useful: a pass can
/// tell a window that has been away for a moment from one that has been away for
/// a minute, and greet them differently. An exit animation is not expressible
/// through it, because by then the engine has stopped drawing the window at all.
pub const TIMES_LANES: usize = 12;

/// A row of [`TIMES_LANES`], in the order the shader arrays read them: `life`,
/// then `state`, then `drag`.
pub type Times = [f32; TIMES_LANES];

/// Nothing has ever happened to this entry.
pub fn no_times(never: f32) -> Times {
    [never; TIMES_LANES]
}
