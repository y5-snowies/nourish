use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_channel_token_base::y5_channel;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_y5_placeholder_state_base::state::PlaceholderState;
use smithay::utils::{Physical, Point, Size};
use std::any::Any;
use uuid::Uuid;
use compositor_orchestration_launch_broadcast_base::broadcast::event::EXECUTED;
use compositor_introspection_execution_launch_types::types::LaunchOutcome;
use compositor_introspection_execution_launch_policy::policy::REQUIRE_PID;
use compositor_y5_placeholder_record_base::placeholder::PlaceholderLaunchToken;

// The slot tokens live with the state (in `placeholder.state`) so the persistence
// document can reference them without a crate cycle; re-export for legacy sites.
pub use compositor_y5_placeholder_state_base::state::{PLACEHOLDER, PLACEHOLDER_MUT};

/// An interactive placeholder geometry update (move/scale drag), migrated from
/// the rim `placeholder.interface::set_visible_geometry`. The placeholder OWNS
/// the slot, so an input system (CanvasSystem) can't write `modify_visible`
/// directly — it announces this; the placeholder system applies the slot half
/// via its buffer AND announces the registry half to the surface system.
/// `position`/`size` are `(x, y)` / `(w, h)` in WORLD-logical space (the space the
/// slot stores). `scale` is the output's fractional scale at the time of the drag,
/// captured by the announcing input system (channel receivers have no
/// `cx.platform`, so they can't read it themselves) — the registry half multiplies
/// the world geometry by it to reach the storage-physical space the iced surface
/// was spawned in (`Transform::into_storage_rect_physical` = world × scale).
#[derive(Clone, Copy)]
pub struct PlaceholderGeometry {
    pub uuid: Uuid,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
    pub scale: f64,
}
y5_channel!(pub PLACEHOLDER_GEOMETRY, PLACEHOLDER_GEOMETRY_TX: PlaceholderGeometry);

/// Announce a placeholder geometry change to the placeholder system. Mirrors the
/// surface system's `announce_iced_button`; cross-crate senders can't reach the
/// channel TX directly, so they call this on the world's router. `position`/`size`
/// are WORLD-logical; `scale` is the current output fractional scale (see
/// [`PlaceholderGeometry`]).
pub fn announce_placeholder_geometry(
    channels: &mut compositor_support_system_channel_router_base::base::ChannelRouter,
    uuid: Uuid,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    scale: f64,
) {
    channels.send(&PLACEHOLDER_GEOMETRY_TX, PlaceholderGeometry { uuid, position, size, scale });
}

enum PlaceholderCmd {
    SetGeometry(Uuid, Option<(i32, i32)>, Option<(i32, i32)>),
    /// Record the activation token / PID for a launched placeholder so the next
    /// matching window restores into it. Set from the Executed launch event.
    SetRestoration(Uuid, PlaceholderLaunchToken),
}
y5_buffer!(PLACEHOLDER_BUF: PlaceholderCmd);

/// Owns the placeholder slot.
#[derive(Default)]
pub struct PlaceholderSystem;

impl System for PlaceholderSystem {
    fn name(&self) -> &'static str {
        "placeholder"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&PLACEHOLDER, PlaceholderState::new());
        builder.receive(&PLACEHOLDER_GEOMETRY, Self::on_geometry);
        // The general launch-completed event; we match on our own placeholder ids.
        builder.receive(&EXECUTED, Self::on_executed);
    }

    /// Persist this world's placeholders (slim launch-plan prior data) into the
    /// per-world `world.placeholder` table; rehydrated at world build.
    fn documents(
        &self,
    ) -> &'static [&'static compositor_support_system_persist_document_entry::base::DocumentEntry] {
        PLACEHOLDER_DOCS
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        match *message.downcast::<PlaceholderCmd>().expect("placeholder buffer type") {
            // Slot half of the rim `set_visible_geometry`: update the visible
            // placeholder's position/size in place. Wrapped in `transact(false, …)`
            // — the geometry is persisted but DEBOUNCED (drags fire many updates/sec,
            // so the disk write batches up to 1s instead of once per frame).
            PlaceholderCmd::SetGeometry(uuid, position, size) => {
                cx.transact(false, |storage| {
                    let state = storage.get_mut(&PLACEHOLDER_MUT);
                    // Visible (window-destroyed) tile — `uuid` is the placeholder id.
                    state.modify_visible(&uuid, |ph| {
                        if let Some((w, h)) = size {
                            ph.size.0 = w;
                            ph.size.1 = h;
                        }
                        if let Some((x, y)) = position {
                            ph.position.0 = x;
                            ph.position.1 = y;
                        }
                    });
                    // Live (window-backed) map placeholder — `uuid` is the window id.
                    // This is the slot half the rim `_reform` kept in sync via
                    // `placeholder.interface::set`; without it the tile spawns at the
                    // pre-drag geometry when the window later closes. The two uuid
                    // namespaces don't collide, so exactly one of these matches.
                    state.modify_present(&uuid, |ph| {
                        if let Some((w, h)) = size {
                            ph.size.0 = w;
                            ph.size.1 = h;
                        }
                        if let Some((x, y)) = position {
                            ph.position.0 = x;
                            ph.position.1 = y;
                        }
                    });
                });
            }
            // Slot half of the launch flow: stamp the restoration token/PID onto
            // the visible placeholder identified by `uuid` (the correlation). The
            // restoration matchers try the token before the PID, so the token-only
            // path (REQUIRE_PID == false) still restores.
            PlaceholderCmd::SetRestoration(uuid, token) => {
                cx.transact(false, |storage| {
                    storage.get_mut(&PLACEHOLDER_MUT).modify_visible(&uuid, |ph| {
                        ph.restoration = Some(token.clone());
                    });
                });
            }
        }
    }
}

impl PlaceholderSystem {
    /// React to the general Executed launch event. Match on our own placeholder
    /// id (the outcome's correlation) and stamp the restoration via the buffer;
    /// outcomes for other originators (correlation `None`/foreign) are ignored.
    fn on_executed(&mut self, cx: &mut SystemCx, outcome: &LaunchOutcome) {
        let Some(uuid) = outcome.correlation else { return };
        if outcome.result.is_err() {
            return;
        }
        // Bootstrap the session-id correlation: `xdg_session_management_v1` has
        // no way for us to TELL a client its session id — the client asks with
        // NULL and we choose the string we send back in `created`. So we note
        // which placeholder this pid was spawned for, and mint that placeholder's
        // uuid when the resulting client asks. Needed once per app; from then on
        // the client hands the id back itself.
        //
        // Uses the raw pid regardless of `REQUIRE_PID` — that toggle exists to
        // exercise the token-only MATCHING path, and this is not matching.
        //
        // Hand over the session id this placeholder ALREADY restores under, so a
        // relaunch re-mints the same string rather than a fresh one. Without it
        // the returning client would be handed a new id every generation and
        // could never find the state it saved under the old one.
        if let Some(pid) = outcome.pid {
            let known = cx
                .storage
                .get(&PLACEHOLDER)
                .visible
                .iter()
                .find(|(ph, _)| ph.uuid == uuid)
                .and_then(|(ph, _)| ph.session.as_ref().map(|s| s.session_id.clone()));
            compositor_support_smithay_state_session_claim::claim::register(
                pid,
                uuid,
                known,
                outcome.token.clone(),
            );
        }
        let token = PlaceholderLaunchToken {
            token: outcome.token.clone(),
            child: if REQUIRE_PID { outcome.pid } else { None },
        };
        cx.write(&PLACEHOLDER_BUF, PlaceholderCmd::SetRestoration(uuid, token));
    }

    /// Announced by CanvasSystem during a placeholder move/scale drag. Apply the
    /// slot half via the buffer; resolve the iced handle from the slot and
    /// announce the registry half to the surface system (rim parity:
    /// `set_visible_geometry` resized/relocated the iced surface too).
    fn on_geometry(&mut self, cx: &mut SystemCx, ev: &PlaceholderGeometry) {
        // Resolve the iced handle id for this placeholder (registry half target).
        let handle = cx.storage.get(&PLACEHOLDER).visible.iter().find_map(|w| {
            if w.0.uuid == ev.uuid { Some(w.1.id) } else { None }
        });

        cx.write(&PLACEHOLDER_BUF, PlaceholderCmd::SetGeometry(ev.uuid, ev.position, ev.size));

        if let Some(handle) = handle {
            // World → storage-physical: the iced surface was spawned at
            // `Transform::into_storage_rect_physical()` (world × scale, camera
            // applied at render). The slot stores world; the registry stores
            // world × scale. Without the scale the surface jumps on the first
            // drag whenever scale != 1 (e.g. fractional-scale winit). See the
            // `scale` note on `PlaceholderGeometry`.
            let s = ev.scale;
            // ROUND THE EDGES, then derive the extent — never round origin and
            // extent independently.
            //
            // Dragging a LEFT or TOP edge changes `position` and `size` together,
            // in opposite directions. Rounded apart, the far edge lands on
            // `round(x·s) + round(w·s)`, which is not `round((x+w)·s)`: the two
            // roundings tick out of step as the drag proceeds and the edge the
            // user is NOT holding wobbles by a pixel. That is the jitter, and it
            // is why only some edges show it — a right/bottom drag leaves `x`
            // alone, so nothing can drift.
            //
            // `motion.rs` already anchors the fixed edge this way in WORLD space
            // for both windows and placeholders; this is the same rule applied to
            // the world -> storage-physical hop, which only placeholders take.
            // (`Transform::into_storage_rect_physical` rounds apart too — uniting
            // them is deferred, so this fixes the path that shows it.)
            let (position, size) = match (ev.position, ev.size) {
                (Some((x, y)), Some((w, h))) => {
                    let left = (x as f64 * s).round() as i32;
                    let top = (y as f64 * s).round() as i32;
                    let right = ((x as f64 + w as f64) * s).round() as i32;
                    let bottom = ((y as f64 + h as f64) * s).round() as i32;
                    (
                        Some(Point::<i32, Physical>::from((left, top))),
                        Some(Size::<i32, Physical>::from((right - left, bottom - top))),
                    )
                }
                // Only one of them moved, so there is no pair to keep consistent:
                // a pure move leaves the extent alone, and a right/bottom drag
                // leaves the origin alone and grows monotonically from it.
                (position, size) => (
                    position.map(|(x, y)| {
                        Point::<i32, Physical>::from((
                            (x as f64 * s).round() as i32,
                            (y as f64 * s).round() as i32,
                        ))
                    }),
                    size.map(|(w, h)| {
                        Size::<i32, Physical>::from((
                            (w as f64 * s).round() as i32,
                            (h as f64 * s).round() as i32,
                        ))
                    }),
                ),
            };
            compositor_y5_surface_system_base::base::announce_placeholder_geometry(
                cx.channels, handle, position, size,
            );
        }
    }
}

static PLACEHOLDER_DOCS: &[&compositor_support_system_persist_document_entry::base::DocumentEntry] =
    &[&compositor_y5_placeholder_persist_doc::base::PLACEHOLDER_DOC];
