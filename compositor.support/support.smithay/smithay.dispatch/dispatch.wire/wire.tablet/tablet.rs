//! External `zwp_tablet_manager_v2` protocol state (tablet-only path).
//!
//! Smithay implements the tablet *tool* protocol but hides its seat instances
//! (`pub(crate)`) and has no *pad* support, so y5 owns the whole tablet stack
//! itself — hand-rolled on the compositor's `Dispatch` state exactly like the
//! `wp_color_management_v1` implementation. All state lives here in `TabletState`
//! (y5 is single-seat, so every client's `zwp_tablet_seat_v2` instance is tracked
//! in one flat list); protocol-object userdata is `()` and cleaned up from the
//! `Dispatch` `destroyed` callbacks. The `<D>` generics keep this leaf crate free
//! of any reference to the concrete `Dispatch` type (which lives downstream).
//!
//! Ported from `vendor/smithay/src/wayland/tablet_manager/{tablet,tablet_seat,
//! tablet_tool}.rs`, minus the `Arc<Mutex<_>>` handles (state is single-owner here)
//! and the per-client cursor scale (tablet-only ⇒ logical surface-local coords).

use std::collections::HashMap;

use smithay::backend::input::{
    ButtonState, TabletToolCapabilities, TabletToolDescriptor,
};
use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::{
    zwp_tablet_pad_dial_v2::ZwpTabletPadDialV2,
    zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2,
    zwp_tablet_pad_ring_v2::{self, ZwpTabletPadRingV2},
    zwp_tablet_pad_strip_v2::{self, ZwpTabletPadStripV2},
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
    zwp_tablet_seat_v2::ZwpTabletSeatV2,
    zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
    zwp_tablet_v2::ZwpTabletV2,
};
use smithay::input::pointer::{CursorImageAttributes, CursorImageStatus};
use smithay::reexports::wayland_server::{
    backend::ObjectId, protocol::wl_surface::WlSurface, Client, Dispatch, DisplayHandle,
    GlobalDispatch, Resource, Weak,
};
use smithay::utils::{Logical, Physical, Point, Serial};
use smithay::wayland::compositor;
use smithay::wayland::seat::CURSOR_IMAGE_ROLE;
use smithay::input::tablet::TabletDescriptor;
use std::sync::Mutex;

pub use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::zwp_tablet_manager_v2::ZwpTabletManagerV2;

/// `zwp_tablet_manager_v2` version we advertise. The protocol's current version is
/// 2 (adds the pad `dial` + tablet `bustype`); v1 clients still bind at v1.
pub const VERSION: u32 = 2;

/// What a pen stroke (tip-down → tip-up) is doing, latched at tip-down by the
/// input session so tip-up releases the matching thing. Opaque here — the routing
/// meaning lives in the input layer.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stroke {
    /// Not in contact (hovering or lifted).
    #[default]
    None,
    /// Contact over a tablet-aware surface → native tool tip forwarded.
    Tablet,
    /// Contact elsewhere → emulated pointer button (pen acts as a mouse).
    Pointer,
    /// Hand (navigation) tool → the tip-drag glide-pans the world (no draw/click).
    Pan,
}

/// Everything the compositor knows about tablets/tools for its single seat.
#[derive(Default)]
pub struct TabletState {
    /// Every bound `zwp_tablet_seat_v2` (one or more per client).
    seats: Vec<Weak<ZwpTabletSeatV2>>,
    /// Known tablet devices → their per-client `zwp_tablet_v2` resources.
    tablets: HashMap<TabletDescriptor, Vec<Weak<ZwpTabletV2>>>,
    /// Known tools → per-client resources + accumulated axis/contact state.
    tools: HashMap<TabletToolDescriptor, Tool>,
    /// Known pad devices (tablet body: buttons/ring/strip), keyed by syspath/name.
    pads: HashMap<String, Pad>,
    /// Current stroke role (latched at tip-down; see [`Stroke`]).
    pub stroke: Stroke,
    /// True while a tool client's `set_cursor` owns the pointer image. Reset on
    /// proximity-out so a stale client cursor surface isn't left as the cursor.
    pub tool_cursor: bool,
    /// Stylus barrel buttons currently emulating a pointer button on a non-tablet
    /// target, as (raw stylus button → emulated pointer button). Lets the matching
    /// release fire even if the pen has since crossed onto a tablet-aware surface,
    /// so a held barrel button never gets stuck.
    emulated_buttons: Vec<(u32, u32)>,
    /// Last pen physical-screen position, for the Hand-tool pan delta. Reset on
    /// proximity-out.
    pub last_pen_screen: Option<Point<f64, Physical>>,
}

/// Per-tool live state, mirroring smithay's `TabletTool` (pending axes flushed on
/// the next motion, per protocol).
#[derive(Default)]
struct Tool {
    instances: Vec<Weak<ZwpTabletToolV2>>,
    focus: Option<WlSurface>,
    is_down: bool,
    pending_pressure: Option<f64>,
    pending_distance: Option<f64>,
    pending_tilt: Option<(f64, f64)>,
    pending_rotation: Option<f64>,
    pending_slider: Option<f64>,
    pending_wheel: Option<(f64, i32)>,
}

/// Advertise the `zwp_tablet_manager_v2` global. Mirrors `wire.color::create_global`.
pub fn create_global<D>(dh: &DisplayHandle)
where
    D: GlobalDispatch<ZwpTabletManagerV2, ()> + 'static,
{
    dh.create_global::<D, ZwpTabletManagerV2, ()>(VERSION, ());
}

// ── resource construction (generic over the concrete state `D`) ─────────────────

fn create_tablet<D>(
    client: &Client,
    dh: &DisplayHandle,
    seat: &ZwpTabletSeatV2,
    desc: &TabletDescriptor,
) -> Option<ZwpTabletV2>
where
    D: Dispatch<ZwpTabletV2, ()> + 'static,
{
    let wl = client
        .create_resource::<ZwpTabletV2, (), D>(dh, seat.version(), ())
        .ok()?;
    seat.tablet_added(&wl);
    wl.name(desc.name.clone());
    if let Some((product, vendor)) = desc.usb_id {
        wl.id(product, vendor);
    }
    if let Some(path) = desc.syspath.as_ref().and_then(|p| p.to_str()) {
        wl.path(path.to_owned());
    }
    wl.done();
    Some(wl)
}

fn create_tool<D>(
    client: &Client,
    dh: &DisplayHandle,
    seat: &ZwpTabletSeatV2,
    desc: &TabletToolDescriptor,
) -> Option<ZwpTabletToolV2>
where
    D: Dispatch<ZwpTabletToolV2, ()> + 'static,
{
    let wl = client
        .create_resource::<ZwpTabletToolV2, (), D>(dh, seat.version(), ())
        .ok()?;
    seat.tool_added(&wl);
    wl._type(desc.tool_type.into());
    // 64-bit hardware ids split hi/lo across two u32 (client re-joins hi<<32|lo).
    wl.hardware_serial((desc.hardware_serial >> 32) as u32, desc.hardware_serial as u32);
    wl.hardware_id_wacom((desc.hardware_id_wacom >> 32) as u32, desc.hardware_id_wacom as u32);
    let caps = desc.capabilities;
    if caps.contains(TabletToolCapabilities::PRESSURE) {
        wl.capability(zwp_tablet_tool_v2::Capability::Pressure);
    }
    if caps.contains(TabletToolCapabilities::DISTANCE) {
        wl.capability(zwp_tablet_tool_v2::Capability::Distance);
    }
    if caps.contains(TabletToolCapabilities::TILT) {
        wl.capability(zwp_tablet_tool_v2::Capability::Tilt);
    }
    if caps.contains(TabletToolCapabilities::ROTATION) {
        wl.capability(zwp_tablet_tool_v2::Capability::Rotation);
    }
    if caps.contains(TabletToolCapabilities::SLIDER) {
        wl.capability(zwp_tablet_tool_v2::Capability::Slider);
    }
    if caps.contains(TabletToolCapabilities::WHEEL) {
        wl.capability(zwp_tablet_tool_v2::Capability::Wheel);
    }
    wl.done();
    Some(wl)
}

impl TabletState {
    // ── seat / device lifecycle ────────────────────────────────────────────────

    /// A client bound a new `zwp_tablet_seat_v2`: replay the known tablets + tools
    /// to it (smithay's `add_instance`).
    pub fn add_seat<D>(&mut self, dh: &DisplayHandle, seat: &ZwpTabletSeatV2, client: &Client)
    where
        D: Dispatch<ZwpTabletV2, ()>
            + Dispatch<ZwpTabletToolV2, ()>
            + Dispatch<ZwpTabletPadV2, ()>
            + Dispatch<ZwpTabletPadGroupV2, ()>
            + Dispatch<ZwpTabletPadRingV2, ()>
            + Dispatch<ZwpTabletPadStripV2, ()>
            + Dispatch<ZwpTabletPadDialV2, ()>
            + 'static,
    {
        for (desc, instances) in self.tablets.iter_mut() {
            if let Some(wl) = create_tablet::<D>(client, dh, seat, desc) {
                instances.push(wl.downgrade());
            }
        }
        for (desc, tool) in self.tools.iter_mut() {
            if let Some(wl) = create_tool::<D>(client, dh, seat, desc) {
                tool.instances.push(wl.downgrade());
            }
        }
        for pad in self.pads.values_mut() {
            if let Some((wl, groups, rings, strips, dials)) = create_pad::<D>(client, dh, seat, &pad.desc) {
                pad.pads.push(wl.downgrade());
                pad.groups.extend(groups);
                pad.rings.extend(rings);
                pad.strips.extend(strips);
                pad.dials.extend(dials);
            }
        }
        self.seats.push(seat.downgrade());
    }

    /// Register a tablet device (hotplug / first tool event) and advertise it to
    /// every bound seat.
    pub fn add_tablet<D>(&mut self, dh: &DisplayHandle, desc: &TabletDescriptor)
    where
        D: Dispatch<ZwpTabletV2, ()> + 'static,
    {
        if self.tablets.contains_key(desc) {
            return;
        }
        let mut instances = Vec::new();
        for seat in self.seats.iter().filter_map(|s| s.upgrade().ok()) {
            if let Ok(client) = dh.get_client(seat.id()) {
                if let Some(wl) = create_tablet::<D>(&client, dh, &seat, desc) {
                    instances.push(wl.downgrade());
                }
            }
        }
        self.tablets.insert(desc.clone(), instances);
    }

    /// Register a tool (first proximity) and advertise it to every bound seat.
    pub fn add_tool<D>(&mut self, dh: &DisplayHandle, desc: &TabletToolDescriptor)
    where
        D: Dispatch<ZwpTabletToolV2, ()> + 'static,
    {
        if self.tools.contains_key(desc) {
            return;
        }
        let mut tool = Tool::default();
        for seat in self.seats.iter().filter_map(|s| s.upgrade().ok()) {
            if let Ok(client) = dh.get_client(seat.id()) {
                if let Some(wl) = create_tool::<D>(&client, dh, &seat, desc) {
                    tool.instances.push(wl.downgrade());
                }
            }
        }
        self.tools.insert(desc.clone(), tool);
    }

    pub fn remove_tablet(&mut self, desc: &TabletDescriptor) {
        self.tablets.remove(desc);
    }

    pub fn has_tablets(&self) -> bool {
        !self.tablets.is_empty()
    }

    pub fn clear_tools(&mut self) {
        self.tools.clear();
    }

    // ── `destroyed` cleanup (called from the Dispatch impls) ────────────────────

    pub fn remove_seat(&mut self, id: &ObjectId) {
        self.seats.retain(|s| &s.id() != id);
    }
    pub fn remove_tablet_resource(&mut self, id: &ObjectId) {
        for v in self.tablets.values_mut() {
            v.retain(|t| &t.id() != id);
        }
    }
    pub fn remove_tool_resource(&mut self, id: &ObjectId) {
        for t in self.tools.values_mut() {
            t.instances.retain(|i| &i.id() != id);
        }
    }

    /// Does the surface's client have a live tablet seat? (analog of
    /// `TouchHandle::client_has_touch`; drives the pen session's Tablet-vs-Pan role.)
    pub fn client_has_tablet(&self, surface: &WlSurface) -> bool {
        self.seats
            .iter()
            .filter_map(|s| s.upgrade().ok())
            .any(|s| s.id().same_client_as(&surface.id()))
    }

    /// Honor a tool client's `set_cursor` (tablet-only path — logical surface-local
    /// coords, no per-client cursor scale). Returns the [`CursorImageStatus`] the rim
    /// should install into `seat.pointer_status`, or `None` to ignore the request when
    /// the requesting tool isn't currently in proximity over one of its own client's
    /// surfaces (mirrors smithay's focus / same-client gate on `wl_pointer::set_cursor`).
    pub fn set_tool_cursor(
        &mut self,
        tool: &ZwpTabletToolV2,
        surface: Option<WlSurface>,
        hotspot: Point<i32, Logical>,
    ) -> Option<CursorImageStatus> {
        let focused = self.tools.values().any(|t| {
            t.instances.iter().any(|i| i.id() == tool.id())
                && t.focus
                    .as_ref()
                    .is_some_and(|f| f.id().same_client_as(&tool.id()))
        });
        if !focused {
            return None;
        }
        match surface {
            Some(surface) => {
                // Reject a surface that already carries an incompatible role.
                if compositor::give_role(&surface, CURSOR_IMAGE_ROLE).is_err()
                    && compositor::get_role(&surface) != Some(CURSOR_IMAGE_ROLE)
                {
                    return None;
                }
                compositor::with_states(&surface, |states| {
                    states
                        .data_map
                        .insert_if_missing_threadsafe(|| Mutex::new(CursorImageAttributes { hotspot: (0, 0).into() }));
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .hotspot = hotspot;
                });
                self.tool_cursor = true;
                Some(CursorImageStatus::Surface(surface))
            }
            None => {
                self.tool_cursor = true;
                Some(CursorImageStatus::Hidden)
            }
        }
    }

    /// Take + clear the "a tool owns the cursor" flag (proximity-out reset).
    pub fn take_tool_cursor(&mut self) -> bool {
        std::mem::take(&mut self.tool_cursor)
    }

    /// Is this tool currently in proximity over a tablet-aware surface? (Its focus
    /// is set only over a `client_has_tablet` surface — see `tool_motion`.) Drives
    /// whether a stylus barrel button forwards natively or emulates a pointer button.
    pub fn tool_has_focus(&self, tool: &TabletToolDescriptor) -> bool {
        self.tools.get(tool).is_some_and(|t| t.focus.is_some())
    }

    /// Record a barrel button now emulating a pointer button (see `emulated_buttons`).
    pub fn push_emulated_button(&mut self, raw: u32, mapped: u32) {
        self.emulated_buttons.push((raw, mapped));
    }

    /// If `raw` was emulating a pointer button, remove and return the mapped button.
    pub fn take_emulated_button(&mut self, raw: u32) -> Option<u32> {
        self.emulated_buttons
            .iter()
            .position(|(r, _)| *r == raw)
            .map(|i| self.emulated_buttons.remove(i).1)
    }

    // ── tool event senders (driven by the input handlers) ──────────────────────

    fn tablet_for(&self, desc: &TabletDescriptor, surface: &WlSurface) -> Option<ZwpTabletV2> {
        self.tablets
            .get(desc)?
            .iter()
            .find(|t| t.id().same_client_as(&surface.id()))
            .and_then(|t| t.upgrade().ok())
    }

    pub fn tool_proximity_in(
        &mut self,
        tool: &TabletToolDescriptor,
        tablet: &TabletDescriptor,
        loc: Point<f64, Logical>,
        focus: (WlSurface, Point<f64, Logical>),
        serial: Serial,
        time: u32,
    ) {
        let wl_tablet = self.tablet_for(tablet, &focus.0);
        if let Some(t) = self.tools.get_mut(tool) {
            t.proximity_in(loc, focus, wl_tablet.as_ref(), serial, time);
        }
    }

    pub fn tool_proximity_out(&mut self, tool: &TabletToolDescriptor, time: u32) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.proximity_out(time);
        }
    }

    pub fn tool_motion(
        &mut self,
        tool: &TabletToolDescriptor,
        tablet: &TabletDescriptor,
        loc: Point<f64, Logical>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        serial: Serial,
        time: u32,
    ) {
        let wl_tablet = focus
            .as_ref()
            .and_then(|f| self.tablet_for(tablet, &f.0));
        if let Some(t) = self.tools.get_mut(tool) {
            t.motion(loc, focus, wl_tablet.as_ref(), serial, time);
        }
    }

    pub fn tool_tip_down(&mut self, tool: &TabletToolDescriptor, serial: Serial, time: u32) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.tip_down(serial, time);
        }
    }
    pub fn tool_tip_up(&mut self, tool: &TabletToolDescriptor, time: u32) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.tip_up(time);
        }
    }
    pub fn tool_button(
        &mut self,
        tool: &TabletToolDescriptor,
        button: u32,
        state: ButtonState,
        serial: Serial,
        time: u32,
    ) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.button(button, state, serial, time);
        }
    }

    pub fn tool_pressure(&mut self, tool: &TabletToolDescriptor, v: f64) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.pending_pressure = Some(v);
        }
    }
    pub fn tool_distance(&mut self, tool: &TabletToolDescriptor, v: f64) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.pending_distance = Some(v);
        }
    }
    pub fn tool_tilt(&mut self, tool: &TabletToolDescriptor, v: (f64, f64)) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.pending_tilt = Some(v);
        }
    }
    pub fn tool_rotation(&mut self, tool: &TabletToolDescriptor, v: f64) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.pending_rotation = Some(v);
        }
    }
    pub fn tool_slider(&mut self, tool: &TabletToolDescriptor, v: f64) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.pending_slider = Some(v);
        }
    }
    pub fn tool_wheel(&mut self, tool: &TabletToolDescriptor, degrees: f64, clicks: i32) {
        if let Some(t) = self.tools.get_mut(tool) {
            t.pending_wheel = Some((degrees, clicks));
        }
    }
}

// ── tablet pad (zwp_tablet_pad_v2) ─────────────────────────────────────────────
// smithay has NO pad support at all, so this is entirely ours. One mode group per
// libinput mode-group, each announcing its rings/strips/buttons/modes.

/// Static description of a pad device, built from its libinput `Device`.
#[derive(Clone, Default)]
pub struct PadDesc {
    pub name: String,
    pub syspath: Option<String>,
    pub buttons: u32,
    pub groups: Vec<GroupDesc>,
}

/// One libinput mode group: which button/ring/strip/dial indices it owns + modes.
#[derive(Clone, Default)]
pub struct GroupDesc {
    pub modes: u32,
    pub buttons: Vec<u32>,
    pub rings: Vec<u32>,
    pub strips: Vec<u32>,
    pub dials: Vec<u32>,
}

/// libinput exposes no dial count; probe `has_dial` over a small range (dials are few).
const MAX_DIALS: u32 = 8;

/// Live pad: the created resources across clients (group/ring/strip/dial carry
/// their index so runtime events route to the right object). Pad events are gated
/// to `focused` — the keyboard-focused client's pad instance — bracketed by
/// enter/leave (y5 is single-seat, so at most one client is focused at a time).
#[derive(Default)]
struct Pad {
    desc: PadDesc,
    pads: Vec<Weak<ZwpTabletPadV2>>,
    groups: Vec<(usize, Weak<ZwpTabletPadGroupV2>)>,
    rings: Vec<(u32, Weak<ZwpTabletPadRingV2>)>,
    strips: Vec<(u32, Weak<ZwpTabletPadStripV2>)>,
    dials: Vec<(u32, Weak<ZwpTabletPadDialV2>)>,
    /// The surface this pad is currently focused on (`enter` sent, `leave` pending).
    focused: Option<WlSurface>,
    /// Current mode of each mode group (indexed by group index), for `mode_switch`
    /// dedup + re-announcing the mode to a newly-focused client.
    group_modes: Vec<u32>,
}

/// Per-client resources created for one pad, returned by `create_pad`.
type PadResources = (
    ZwpTabletPadV2,
    Vec<(usize, Weak<ZwpTabletPadGroupV2>)>,
    Vec<(u32, Weak<ZwpTabletPadRingV2>)>,
    Vec<(u32, Weak<ZwpTabletPadStripV2>)>,
    Vec<(u32, Weak<ZwpTabletPadDialV2>)>,
);

/// Build a [`PadDesc`] from a libinput pad device (called on `DeviceAdded`).
pub fn pad_desc_from_device(device: &smithay::reexports::input::Device) -> PadDesc {
    let buttons = device.tablet_pad_number_of_buttons().max(0) as u32;
    let rings = device.tablet_pad_number_of_rings().max(0) as u32;
    let strips = device.tablet_pad_number_of_strips().max(0) as u32;
    let num_groups = device.tablet_pad_number_of_mode_groups().max(0) as u32;
    let mut groups = Vec::new();
    for gi in 0..num_groups {
        let Some(g) = device.tablet_pad_mode_group(gi) else { continue };
        groups.push(GroupDesc {
            modes: g.number_of_modes(),
            buttons: (0..buttons).filter(|&b| g.has_button(b)).collect(),
            rings: (0..rings).filter(|&r| g.has_ring(r)).collect(),
            strips: (0..strips).filter(|&s| g.has_strip(s)).collect(),
            dials: (0..MAX_DIALS).filter(|&d| g.has_dial(d)).collect(),
        });
    }
    // Fallback: a single group owning everything (pads that report no mode groups).
    if groups.is_empty() {
        groups.push(GroupDesc {
            modes: 1,
            buttons: (0..buttons).collect(),
            rings: (0..rings).collect(),
            strips: (0..strips).collect(),
            dials: Vec::new(),
        });
    }
    PadDesc {
        name: device.name().to_string(),
        syspath: Some(device.sysname().to_string()),
        buttons,
        groups,
    }
}

fn create_pad<D>(
    client: &Client,
    dh: &DisplayHandle,
    seat: &ZwpTabletSeatV2,
    desc: &PadDesc,
) -> Option<PadResources>
where
    D: Dispatch<ZwpTabletPadV2, ()>
        + Dispatch<ZwpTabletPadGroupV2, ()>
        + Dispatch<ZwpTabletPadRingV2, ()>
        + Dispatch<ZwpTabletPadStripV2, ()>
        + Dispatch<ZwpTabletPadDialV2, ()>
        + 'static,
{
    let pad = client
        .create_resource::<ZwpTabletPadV2, (), D>(dh, seat.version(), ())
        .ok()?;
    seat.pad_added(&pad);
    let (mut groups, mut rings, mut strips, mut dials) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (gi, g) in desc.groups.iter().enumerate() {
        let group = client
            .create_resource::<ZwpTabletPadGroupV2, (), D>(dh, pad.version(), ())
            .ok()?;
        pad.group(&group);
        groups.push((gi, group.downgrade()));
        for &r in &g.rings {
            if let Ok(ring) = client.create_resource::<ZwpTabletPadRingV2, (), D>(dh, pad.version(), ()) {
                group.ring(&ring);
                rings.push((r, ring.downgrade()));
            }
        }
        for &s in &g.strips {
            if let Ok(strip) = client.create_resource::<ZwpTabletPadStripV2, (), D>(dh, pad.version(), ()) {
                group.strip(&strip);
                strips.push((s, strip.downgrade()));
            }
        }
        // The `dial` event + interface are protocol v2 — only a v2 client's group.
        if group.version() >= 2 {
            for &d in &g.dials {
                if let Ok(dial) = client.create_resource::<ZwpTabletPadDialV2, (), D>(dh, pad.version(), ()) {
                    group.dial(&dial);
                    dials.push((d, dial.downgrade()));
                }
            }
        }
        // group.buttons is a wl_array of native-endian u32 button indices.
        let mut bytes = Vec::with_capacity(g.buttons.len() * 4);
        for b in &g.buttons {
            bytes.extend_from_slice(&b.to_ne_bytes());
        }
        group.buttons(bytes);
        group.modes(g.modes);
        group.done();
    }
    if desc.buttons > 0 {
        pad.buttons(desc.buttons);
    }
    if let Some(path) = &desc.syspath {
        pad.path(path.clone());
    }
    pad.done();
    Some((pad, groups, rings, strips, dials))
}

impl TabletState {
    /// Register a pad device and advertise it to every bound seat.
    pub fn add_pad<D>(&mut self, dh: &DisplayHandle, key: String, desc: PadDesc)
    where
        D: Dispatch<ZwpTabletPadV2, ()>
            + Dispatch<ZwpTabletPadGroupV2, ()>
            + Dispatch<ZwpTabletPadRingV2, ()>
            + Dispatch<ZwpTabletPadStripV2, ()>
            + Dispatch<ZwpTabletPadDialV2, ()>
            + 'static,
    {
        if self.pads.contains_key(&key) {
            return;
        }
        let mut pad = Pad {
            group_modes: vec![0; desc.groups.len()],
            desc,
            ..Default::default()
        };
        for seat in self.seats.iter().filter_map(|s| s.upgrade().ok()) {
            if let Ok(client) = dh.get_client(seat.id()) {
                if let Some((wl, groups, rings, strips, dials)) = create_pad::<D>(&client, dh, &seat, &pad.desc) {
                    pad.pads.push(wl.downgrade());
                    pad.groups.extend(groups);
                    pad.rings.extend(rings);
                    pad.strips.extend(strips);
                    pad.dials.extend(dials);
                }
            }
        }
        self.pads.insert(key, pad);
    }

    pub fn remove_pad(&mut self, key: &str) {
        self.pads.remove(key);
    }

    /// A tablet resource on the surface's client, if any (single-seat fallback for a
    /// pad's `enter` `tablet` argument — pads and tablets share a device group, but
    /// libinput doesn't expose that mapping to us).
    fn first_tablet_for(&self, surface: &WlSurface) -> Option<ZwpTabletV2> {
        self.tablets
            .values()
            .flatten()
            .find(|t| t.id().same_client_as(&surface.id()))
            .and_then(|t| t.upgrade().ok())
    }

    /// Move tablet-pad focus to the keyboard-focused surface (`None` ⇒ clear). Sends
    /// `leave` to the previously-focused client's pad and `enter` to the new one, so
    /// pad button/ring/strip/dial/mode events are gated to the focused client. Driven
    /// from `SeatHandler::focus_changed`.
    pub fn set_pad_focus(&mut self, focused: Option<WlSurface>, serial: Serial) {
        let tablet = focused.as_ref().and_then(|s| self.first_tablet_for(s));
        for pad in self.pads.values_mut() {
            pad.set_focus(focused.as_ref(), tablet.as_ref(), serial);
        }
    }

    /// The libinput-reported mode of a pad mode group changed → announce it to the
    /// focused client (deduped: a no-op when the mode is unchanged).
    pub fn pad_mode_switch(&mut self, key: &str, group: usize, mode: u32, serial: Serial, time: u32) {
        let Some(pad) = self.pads.get_mut(key) else { return };
        if group >= pad.group_modes.len() {
            pad.group_modes.resize(group + 1, 0);
        }
        if pad.group_modes[group] == mode {
            return;
        }
        pad.group_modes[group] = mode;
        let Some(surface) = pad.focused.clone() else { return };
        for wl in pad
            .groups
            .iter()
            .filter(|(g, _)| *g == group)
            .filter_map(|(_, w)| w.upgrade().ok())
            .filter(|wl| wl.id().same_client_as(&surface.id()))
        {
            wl.mode_switch(time, serial.into(), mode);
        }
    }

    /// A pad button changed. Delivered only to the focused client.
    pub fn pad_button(&self, key: &str, button: u32, pressed: bool, time: u32) {
        let Some(pad) = self.pads.get(key) else { return };
        let Some(wl) = pad.focused_pad() else { return };
        let state = if pressed {
            zwp_tablet_pad_v2::ButtonState::Pressed
        } else {
            zwp_tablet_pad_v2::ButtonState::Released
        };
        wl.button(time, button, state);
    }

    /// A pad ring moved (`degrees < 0` ⇒ finger lifted → stop). Focused client only.
    pub fn pad_ring(&self, key: &str, ring: u32, degrees: f64, finger: bool, time: u32) {
        let Some(pad) = self.pads.get(key) else { return };
        let Some(surface) = pad.focused.as_ref() else { return };
        for wl in pad
            .rings
            .iter()
            .filter(|(n, _)| *n == ring)
            .filter_map(|(_, w)| w.upgrade().ok())
            .filter(|wl| wl.id().same_client_as(&surface.id()))
        {
            if finger {
                wl.source(zwp_tablet_pad_ring_v2::Source::Finger);
            }
            if degrees >= 0.0 {
                wl.angle(degrees);
            } else {
                wl.stop();
            }
            wl.frame(time);
        }
    }

    /// A pad strip moved (`position < 0` ⇒ finger lifted → stop). `position` 0..1.
    /// Focused client only.
    pub fn pad_strip(&self, key: &str, strip: u32, position: f64, finger: bool, time: u32) {
        let Some(pad) = self.pads.get(key) else { return };
        let Some(surface) = pad.focused.as_ref() else { return };
        for wl in pad
            .strips
            .iter()
            .filter(|(n, _)| *n == strip)
            .filter_map(|(_, w)| w.upgrade().ok())
            .filter(|wl| wl.id().same_client_as(&surface.id()))
        {
            if finger {
                wl.source(zwp_tablet_pad_strip_v2::Source::Finger);
            }
            if position >= 0.0 {
                wl.position((position.clamp(0.0, 1.0) * 65535.0).round() as u32);
            } else {
                wl.stop();
            }
            wl.frame(time);
        }
    }

    /// A pad dial turned. `v120` is high-res delta (120 units per detent). Focused
    /// client only.
    pub fn pad_dial(&self, key: &str, dial: u32, v120: i32, time: u32) {
        let Some(pad) = self.pads.get(key) else { return };
        let Some(surface) = pad.focused.as_ref() else { return };
        for wl in pad
            .dials
            .iter()
            .filter(|(n, _)| *n == dial)
            .filter_map(|(_, w)| w.upgrade().ok())
            .filter(|wl| wl.id().same_client_as(&surface.id()))
        {
            wl.delta(v120);
            wl.frame(time);
        }
    }

    pub fn remove_pad_resource(&mut self, id: &ObjectId) {
        for pad in self.pads.values_mut() {
            pad.pads.retain(|p| &p.id() != id);
            pad.groups.retain(|(_, g)| &g.id() != id);
            pad.rings.retain(|(_, r)| &r.id() != id);
            pad.strips.retain(|(_, s)| &s.id() != id);
            pad.dials.retain(|(_, d)| &d.id() != id);
        }
    }
}

impl Pad {
    /// The pad resource owned by `surface`'s client.
    fn instance_for(&self, surface: &WlSurface) -> Option<ZwpTabletPadV2> {
        self.pads
            .iter()
            .find(|p| p.id().same_client_as(&surface.id()))
            .and_then(|p| p.upgrade().ok())
    }

    /// The pad resource on the currently-focused client (event-gating target).
    fn focused_pad(&self) -> Option<ZwpTabletPadV2> {
        self.instance_for(self.focused.as_ref()?)
    }

    /// Update this pad's focus: `leave` the old surface, `enter` the new one (which
    /// needs a `zwp_tablet_v2` on that client — `enter` carries it), then re-announce
    /// each group's current mode so the newly-focused client isn't stale. `focused`
    /// is only set once `enter` actually goes out (no tablet resource ⇒ can't enter ⇒
    /// stays unfocused, so no pad events leak to a client that never got `enter`).
    fn set_focus(&mut self, focused: Option<&WlSurface>, tablet: Option<&ZwpTabletV2>, serial: Serial) {
        if self.focused.as_ref() == focused {
            return;
        }
        if let Some(old) = self.focused.take() {
            if let Some(wl) = self.instance_for(&old) {
                wl.leave(serial.into(), &old);
            }
        }
        if let (Some(surface), Some(tablet)) = (focused, tablet) {
            if let Some(wl) = self.instance_for(surface) {
                wl.enter(serial.into(), tablet, surface);
                for (g, w) in &self.groups {
                    let mode = self.group_modes.get(*g).copied().unwrap_or(0);
                    if mode == 0 {
                        continue; // 0 is the default; clients assume it on enter.
                    }
                    if let Ok(group) = w.upgrade() {
                        if group.id().same_client_as(&surface.id()) {
                            group.mode_switch(0, serial.into(), mode);
                        }
                    }
                }
                self.focused = Some(surface.clone());
            }
        }
    }
}

impl Tool {
    fn instance_for(&self, surface: &WlSurface) -> Option<ZwpTabletToolV2> {
        self.instances
            .iter()
            .find(|i| i.id().same_client_as(&surface.id()))
            .and_then(|i| i.upgrade().ok())
    }

    fn proximity_in(
        &mut self,
        loc: Point<f64, Logical>,
        (focus, origin): (WlSurface, Point<f64, Logical>),
        wl_tablet: Option<&ZwpTabletV2>,
        serial: Serial,
        time: u32,
    ) {
        if let (Some(wl_tool), Some(wl_tablet)) = (self.instance_for(&focus), wl_tablet) {
            wl_tool.proximity_in(serial.into(), wl_tablet, &focus);
            // proximity_in must be followed by a motion+frame (protocol requirement).
            let rel = loc - origin;
            wl_tool.motion(rel.x, rel.y);
            wl_tool.frame(time);
        }
        self.focus = Some(focus);
    }

    fn proximity_out(&mut self, time: u32) {
        if let Some(focus) = self.focus.clone() {
            if let Some(wl_tool) = self.instance_for(&focus) {
                if self.is_down {
                    wl_tool.pressure(0);
                    wl_tool.up();
                    self.is_down = false;
                }
                wl_tool.proximity_out();
                wl_tool.frame(time);
            }
        }
        self.focus = None;
    }

    fn tip_down(&mut self, serial: Serial, time: u32) {
        if let Some(focus) = self.focus.clone() {
            if let Some(wl_tool) = self.instance_for(&focus) {
                if !self.is_down {
                    wl_tool.down(serial.into());
                    wl_tool.frame(time);
                }
            }
        }
        self.is_down = true;
    }

    fn tip_up(&mut self, time: u32) {
        if let Some(focus) = self.focus.clone() {
            if let Some(wl_tool) = self.instance_for(&focus) {
                if self.is_down {
                    wl_tool.pressure(0);
                    wl_tool.up();
                    wl_tool.frame(time);
                }
            }
        }
        self.is_down = false;
    }

    fn motion(
        &mut self,
        loc: Point<f64, Logical>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        wl_tablet: Option<&ZwpTabletV2>,
        serial: Serial,
        time: u32,
    ) {
        match (focus, self.focus.clone()) {
            (Some(focus), Some(prev)) if focus.0 == prev => {
                if let Some(wl_tool) = self.instance_for(&focus.0) {
                    let rel = loc - focus.1;
                    wl_tool.motion(rel.x, rel.y);
                    if let Some(v) = self.pending_pressure.take() {
                        wl_tool.pressure((v * 65535.0).round() as u32);
                    }
                    if let Some(v) = self.pending_distance.take() {
                        wl_tool.distance((v * 65535.0).round() as u32);
                    }
                    if let Some((x, y)) = self.pending_tilt.take() {
                        wl_tool.tilt(x, y);
                    }
                    if let Some(v) = self.pending_slider.take() {
                        wl_tool.slider((v * 65535.0).round() as i32);
                    }
                    if let Some(v) = self.pending_rotation.take() {
                        wl_tool.rotation(v);
                    }
                    if let Some((degrees, clicks)) = self.pending_wheel.take() {
                        wl_tool.wheel(degrees, clicks);
                    }
                    wl_tool.frame(time);
                }
            }
            // Focus changed: leave the old surface, enter the new one.
            (Some(focus), Some(_)) => {
                self.proximity_out(time);
                self.proximity_in(loc, focus, wl_tablet, serial, time);
            }
            (Some(focus), None) => self.proximity_in(loc, focus, wl_tablet, serial, time),
            (None, _) => self.proximity_out(time),
        }
    }

    fn button(&self, button: u32, state: ButtonState, serial: Serial, time: u32) {
        if let Some(focus) = self.focus.clone() {
            if let Some(wl_tool) = self.instance_for(&focus) {
                wl_tool.button(serial.into(), button, state.into());
                wl_tool.frame(time);
            }
        }
    }
}
