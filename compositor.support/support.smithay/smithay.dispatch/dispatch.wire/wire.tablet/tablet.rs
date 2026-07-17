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
    zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2,
    zwp_tablet_pad_ring_v2::{self, ZwpTabletPadRingV2},
    zwp_tablet_pad_strip_v2::{self, ZwpTabletPadStripV2},
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
    zwp_tablet_seat_v2::ZwpTabletSeatV2,
    zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
    zwp_tablet_v2::ZwpTabletV2,
};
use smithay::reexports::wayland_server::{
    backend::ObjectId, protocol::wl_surface::WlSurface, Client, Dispatch, DisplayHandle,
    GlobalDispatch, Resource, Weak,
};
use smithay::utils::{Logical, Point, Serial};
use smithay::wayland::tablet_manager::TabletDescriptor;

pub use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::zwp_tablet_manager_v2::ZwpTabletManagerV2;

/// `zwp_tablet_manager_v2` version we advertise (matches smithay + the protocol).
pub const VERSION: u32 = 1;

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
            if let Some((wl, rings, strips)) = create_pad::<D>(client, dh, seat, &pad.desc) {
                pad.pads.push(wl.downgrade());
                pad.rings.extend(rings);
                pad.strips.extend(strips);
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

/// One libinput mode group: which button/ring/strip indices it owns + its modes.
#[derive(Clone, Default)]
pub struct GroupDesc {
    pub modes: u32,
    pub buttons: Vec<u32>,
    pub rings: Vec<u32>,
    pub strips: Vec<u32>,
}

/// Live pad: the created resources across clients (ring/strip carry their index so
/// runtime events route to the right object). Focus/enter-leave is not modelled —
/// button/ring/strip events broadcast to every bound pad (y5 is single-seat).
#[derive(Default)]
struct Pad {
    desc: PadDesc,
    pads: Vec<Weak<ZwpTabletPadV2>>,
    rings: Vec<(u32, Weak<ZwpTabletPadRingV2>)>,
    strips: Vec<(u32, Weak<ZwpTabletPadStripV2>)>,
}

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
        });
    }
    // Fallback: a single group owning everything (pads that report no mode groups).
    if groups.is_empty() {
        groups.push(GroupDesc {
            modes: 1,
            buttons: (0..buttons).collect(),
            rings: (0..rings).collect(),
            strips: (0..strips).collect(),
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
) -> Option<(ZwpTabletPadV2, Vec<(u32, Weak<ZwpTabletPadRingV2>)>, Vec<(u32, Weak<ZwpTabletPadStripV2>)>)>
where
    D: Dispatch<ZwpTabletPadV2, ()>
        + Dispatch<ZwpTabletPadGroupV2, ()>
        + Dispatch<ZwpTabletPadRingV2, ()>
        + Dispatch<ZwpTabletPadStripV2, ()>
        + 'static,
{
    let pad = client
        .create_resource::<ZwpTabletPadV2, (), D>(dh, seat.version(), ())
        .ok()?;
    seat.pad_added(&pad);
    let (mut rings, mut strips) = (Vec::new(), Vec::new());
    for g in &desc.groups {
        let group = client
            .create_resource::<ZwpTabletPadGroupV2, (), D>(dh, pad.version(), ())
            .ok()?;
        pad.group(&group);
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
    Some((pad, rings, strips))
}

impl TabletState {
    /// Register a pad device and advertise it to every bound seat.
    pub fn add_pad<D>(&mut self, dh: &DisplayHandle, key: String, desc: PadDesc)
    where
        D: Dispatch<ZwpTabletPadV2, ()>
            + Dispatch<ZwpTabletPadGroupV2, ()>
            + Dispatch<ZwpTabletPadRingV2, ()>
            + Dispatch<ZwpTabletPadStripV2, ()>
            + 'static,
    {
        if self.pads.contains_key(&key) {
            return;
        }
        let mut pad = Pad {
            desc,
            ..Default::default()
        };
        for seat in self.seats.iter().filter_map(|s| s.upgrade().ok()) {
            if let Ok(client) = dh.get_client(seat.id()) {
                if let Some((wl, rings, strips)) = create_pad::<D>(&client, dh, &seat, &pad.desc) {
                    pad.pads.push(wl.downgrade());
                    pad.rings.extend(rings);
                    pad.strips.extend(strips);
                }
            }
        }
        self.pads.insert(key, pad);
    }

    pub fn remove_pad(&mut self, key: &str) {
        self.pads.remove(key);
    }

    /// A pad button changed. Broadcast to every bound instance.
    pub fn pad_button(&self, key: &str, button: u32, pressed: bool, time: u32) {
        let Some(pad) = self.pads.get(key) else { return };
        let state = if pressed {
            zwp_tablet_pad_v2::ButtonState::Pressed
        } else {
            zwp_tablet_pad_v2::ButtonState::Released
        };
        for wl in pad.pads.iter().filter_map(|p| p.upgrade().ok()) {
            wl.button(time, button, state);
        }
    }

    /// A pad ring moved (`degrees < 0` ⇒ finger lifted → stop).
    pub fn pad_ring(&self, key: &str, ring: u32, degrees: f64, finger: bool, time: u32) {
        let Some(pad) = self.pads.get(key) else { return };
        for wl in pad
            .rings
            .iter()
            .filter(|(n, _)| *n == ring)
            .filter_map(|(_, w)| w.upgrade().ok())
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
    pub fn pad_strip(&self, key: &str, strip: u32, position: f64, finger: bool, time: u32) {
        let Some(pad) = self.pads.get(key) else { return };
        for wl in pad
            .strips
            .iter()
            .filter(|(n, _)| *n == strip)
            .filter_map(|(_, w)| w.upgrade().ok())
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

    pub fn remove_pad_resource(&mut self, id: &ObjectId) {
        for pad in self.pads.values_mut() {
            pad.pads.retain(|p| &p.id() != id);
            pad.rings.retain(|(_, r)| &r.id() != id);
            pad.strips.retain(|(_, s)| &s.id() != id);
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
