//! Hotplug reconciliation: converge the driven outputs to the connected set.
//! `reconcile` (called from the udev hotplug route + at startup) adds a pipe per
//! newly-connected monitor (`add_output`), drops pipes whose connector vanished,
//! fails the primary over to another connected monitor if its own vanished, and
//! goes dark when nothing is connected. Also holds the shared bring-up helper
//! (`bring_up`) and the rim-facing snapshot writer (`write_snapshots`).
//!
//! (The former user-initiated ACTIVE-OUTPUT switch transaction lived here too; it
//! was a single-output construct — tear the sole pipe down and re-light a chosen
//! connector — that doesn't fit independently-driven multi-output, and mode changes
//! now go per-pipe through `display.mode`. It has been removed.)
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_kernel_graphic_preference_output_profile::profile::{self, ModeRequest};
use compositor_orchestration_event_output_base::output::OutputChange;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_driver_lid_base::base::{DISPLAY_OFF_MUT, DISPLAY_SNAPSHOT_MUT};
use compositor_orchestration_driver_output_base::base::{
    ModeInfo, OutputModesSnapshot, OutputsSnapshot, OUTPUTS_SNAPSHOT_MUT, OUTPUT_MODES_SNAPSHOT_MUT,
    OUTPUT_RECONCILE_REQUEST, OUTPUT_RECONCILE_REQUEST_MUT,
};
use smithay::backend::drm::DrmDevice;
use smithay::output::Mode;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::drm::control::{connector, crtc, Mode as DrmMode};
use std::cell::RefCell;
use std::rc::Rc;

type Ctx = Rc<RefCell<NativeRenderContext>>;

/// If the settings window requested a reconcile (activate/deactivate changed the
/// active set), run one — DEFERRED onto a one-shot loop timer so the modeset never
/// runs inside a vblank/render callback (same rule as `display.mode::drain`). No-op
/// when no request is pending.
pub fn drain_reconcile(state: &mut Loop, ctx_rc: &Ctx) {
    if !*state.inner.kernel.get(&OUTPUT_RECONCILE_REQUEST) {
        return;
    }
    *state.inner.kernel.get_mut(&OUTPUT_RECONCILE_REQUEST_MUT) = false;
    let ctx = ctx_rc.clone();
    state
        .loop_handle
        .insert_source(Timer::immediate(), move |_, _, state: &mut Loop| {
            reconcile(state, &ctx);
            TimeoutAction::Drop
        })
        .expect("reconcile deferral timer registration failed");
}

fn mode_info(m: DrmMode) -> ModeInfo {
    ModeInfo { width: m.size().0, height: m.size().1, refresh_mhz: m.vrefresh() * 1000 }
}

/// The EDID identity key ("make model serial") for a connector — the same key the
/// picker selects with and the settings-editor persists.
fn identity_key(drm: &DrmDevice, info: &connector::Info) -> String {
    let raw = compositor_kernel_drm_edid_parse_base::parse::read(drm, info);
    let parsed = raw.as_ref().and_then(compositor_kernel_drm_edid_parse_base::parse::parse);
    compositor_kernel_drm_edid_identity_base::identity::identity(
        parsed.as_ref(),
        &format!("{:?}-{}", info.interface(), info.interface_id()),
    )
    .key()
}

/// The connected connector whose EDID identity matches `key`.
fn find_target(drm: &DrmDevice, key: &str) -> Option<connector::Info> {
    let res = compositor_kernel_drm_connector_scan_base::scan::resources(drm);
    let infos = compositor_kernel_drm_connector_scan_base::scan::connectors(drm, &res);
    infos
        .into_iter()
        .find(|i| i.state() == connector::State::Connected && identity_key(drm, i) == key)
}

/// Tear down the current pipe (freeing its CRTC) and bring `target` up as the primary
/// output, reusing the smithay `Output`. `busy` lists CRTCs held by OTHER live pipes
/// (surviving secondaries) so the rebuild can't claim a CRTC one of them is scanning
/// out. On success the context reflects the new connector/mode. On failure
/// `ctx.pipe().drm_output` is left `None` — the caller must rebuild a working output
/// (render frames skip while it is `None`).
fn bring_up(ctx: &mut NativeRenderContext, target: &connector::Info, requested: Option<ModeInfo>, busy: &[crtc::Handle]) -> Result<(), String> {
    // Drop the current output FIRST so its CRTC/bandwidth is free for the target
    // (the atomic modeset of a second simultaneous pipe is rejected).
    ctx.pipe_mut().drm_output = None;
    let built = compositor_kernel_native_context_display_build::build::build(
        &ctx.drm_output_manager,
        &ctx.gpu_binding,
        &ctx.pipe().output,
        busy,
        target,
        requested,
    )?;
    let env = compositor_developer_environment_config_base::base::get();
    let new_hdr_active = env.hdr && built.hdr.hdr_capable() && ctx.vulkan_mode;
    let new_mode = Mode::from(built.drm_mode);
    ctx.pipe_mut().drm_output = Some(built.drm_output);
    // The fail-over may land the primary on a DIFFERENT CRTC than it held before.
    // `crtc` is the vblank routing key (frame.rs) AND the `busy` exclusion key that
    // `add_output` uses to avoid double-claiming a CRTC — leaving it stale routes the
    // new CRTC's vblanks nowhere and lets a later reactivation steal the primary's
    // real CRTC (both-black). Keep it truthful.
    ctx.pipe_mut().crtc = built.crtc;
    ctx.pipe_mut().mode = new_mode;
    ctx.pipe_mut().current_drm_mode = built.drm_mode;
    ctx.pipe_mut().modes = built.modes;
    ctx.pipe_mut().connector = built.connector;
    ctx.pipe_mut().hdr_caps = built.hdr;
    ctx.pipe_mut().hdr_active = new_hdr_active;
    ctx.pipe_mut().hdr_signalled = false;
    // Fresh scanout target — no page-flip is pending on it, so clear any stale in-flight
    // flag left by the torn-down output (else the render loop would skip the rebuilt pipe
    // and it would never flip / never get a vblank to clear the flag).
    ctx.pipe_mut().in_flight = false;
    ctx.pipe().output.change_current_state(Some(new_mode), None, None, None);
    Ok(())
}

/// Rewrite the rim-facing snapshots (full connector list + active modes + lid) for
/// the connector now driving the compositor.
fn write_snapshots(state: &mut Loop, ctx: &NativeRenderContext) {
    let active = ctx.pipe().connector;
    // Current mode of every DRIVEN pipe, so each connected monitor reports its own
    // `current` in the snapshot (multi-output), not just the primary.
    let lit: Vec<(connector::Handle, ModeInfo)> = ctx
        .outputs
        .iter()
        .filter(|p| p.drm_output.is_some())
        .map(|p| (p.connector, mode_info(p.current_drm_mode)))
        .collect();
    let snap = {
        let mgr = ctx.drm_output_manager.borrow();
        let drm = mgr.device();
        let snap = compositor_kernel_native_context_display_enumerate::enumerate::enumerate(drm, active, &lit);
        let display_snap = compositor_kernel_native_context_display_base::base::compute(drm, active);
        *state.inner.kernel.get_mut(&DISPLAY_SNAPSHOT_MUT) = display_snap;
        snap
    };
    if let Some(d) = snap.displays.iter().find(|d| d.active) {
        *state.inner.kernel.get_mut(&OUTPUT_MODES_SNAPSHOT_MUT) =
            OutputModesSnapshot { edid_key: d.edid_key.clone(), current: d.current, available: d.available.clone() };
    }
    *state.inner.kernel.get_mut(&OUTPUTS_SNAPSHOT_MUT) = snap;
}

/// The preferred-monitor key (the FIRST output profile's identity — same default-output
/// rule startup uses), if set.
fn preferred_key() -> Option<String> {
    profile::get().into_iter().next().and_then(|p| p.identity)
}

/// Resolve the mode to bring `info` up at FROM PREFERENCES — its per-output profile's
/// advertised mode, else the global default mode — mirroring startup. `None` lets the
/// builder fall back to the default-policy mode.
fn pref_mode(drm: &DrmDevice, info: &connector::Info) -> Option<ModeInfo> {
    let to_info = |m: &ModeRequest| match m {
        ModeRequest::Advertised { width, height, refresh_mhz } => {
            Some(ModeInfo { width: *width, height: *height, refresh_mhz: *refresh_mhz })
        }
        _ => None,
    };
    let key = identity_key(drm, info);
    let profiles = profile::get();
    profiles
        .iter()
        .find(|p| p.identity.as_deref() == Some(key.as_str()))
        .or_else(|| profiles.iter().find(|p| p.identity.is_none()))
        .and_then(|p| p.mode.as_ref())
        .and_then(to_info)
        .or_else(|| profile::default_mode().as_ref().and_then(to_info))
}

/// Pick the connector to drive among the connected ones: the preferred monitor if
/// present, else the first connected — exactly the startup `connector.select` policy.
fn pick_target(drm: &DrmDevice, connected: &[connector::Info]) -> Option<connector::Info> {
    if let Some(key) = preferred_key() {
        if let Some(c) = connected.iter().find(|c| identity_key(drm, c) == key) {
            return Some(c.clone());
        }
    }
    connected.first().cloned()
}

/// Tear the display down and idle the render loop until a monitor returns.
fn go_dark(state: &mut Loop, ctx: &mut NativeRenderContext) {
    ctx.pipe_mut().drm_output = None;
    *state.inner.kernel.get_mut(&DISPLAY_OFF_MUT) = true;
    *state.inner.kernel.get_mut(&OUTPUT_MODES_SNAPSHOT_MUT) = OutputModesSnapshot::default();
    *state.inner.kernel.get_mut(&OUTPUTS_SNAPSHOT_MUT) = OutputsSnapshot::default();
}

/// Bring a NEW connected monitor online as an ADDITIONAL output: build a fresh
/// smithay `Output` + a second pipe on a free CRTC (validating the second atomic
/// modeset over the fallback chain — `Err` on no free CRTC / bandwidth / modeset
/// failure, so it fails SOFT), place it to the right of the existing outputs, map
/// it into the `Space`, publish its `wl_output`, and push its `OutputPipe`.
fn add_output(
    state: &mut Loop,
    ctx: &mut NativeRenderContext,
    target: &connector::Info,
    requested: Option<ModeInfo>,
) -> Result<(), String> {
    // Fresh smithay Output from this connector's EDID identity.
    let output = {
        let mgr = ctx.drm_output_manager.borrow();
        let drm = mgr.device();
        let raw = compositor_kernel_drm_edid_parse_base::parse::read(drm, target);
        let parsed = raw.as_ref().and_then(compositor_kernel_drm_edid_parse_base::parse::parse);
        let identity = compositor_kernel_drm_edid_identity_base::identity::identity(
            parsed.as_ref(),
            &format!("{:?}-{}", target.interface(), target.interface_id()),
        );
        compositor_kernel_drm_output_physical_base::physical::create(target, &identity)
    };
    // Second pipe on a free CRTC (excluding the ones already lit).
    let busy: Vec<crtc::Handle> = ctx.outputs.iter().map(|p| p.crtc).collect();
    let built = compositor_kernel_native_context_display_build::build::build(
        &ctx.drm_output_manager,
        &ctx.gpu_binding,
        &output,
        &busy,
        target,
        requested,
    )?;
    // Place to the right of the existing outputs (non-overlapping horizontal tiling,
    // matching `graphic.preference.layout.output::tile_positions`).
    let x: i32 = ctx.outputs.iter().map(|p| p.mode.size.w).sum();
    let mode = Mode::from(built.drm_mode);
    compositor_kernel_drm_output_physical_base::physical::apply_initial_state(&output, mode, None, (x, 0));
    // Keep the GlobalId so the pipe's `wl_output` can be destroyed when this output is
    // pruned (disconnect/deactivate) — otherwise a stale global lingers and re-adding
    // the monitor advertises a duplicate.
    let global = output.create_global::<compositor_support_smithay_dispatch_state_base::state::Dispatch>(
        &ctx.display_handle,
    );
    state.inner.space_state_mut().state.map_output(&output, (x, 0));
    let damage_tracker = smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);
    let env = compositor_developer_environment_config_base::base::get();
    let hdr_active = env.hdr && built.hdr.hdr_capable() && ctx.vulkan_mode;
    info!(
        "add_output: connector={:?} crtc={:?} mode={}x{} pos=({}, 0) → {} outputs total",
        built.connector,
        built.crtc,
        built.drm_mode.size().0,
        built.drm_mode.size().1,
        x,
        ctx.outputs.len() + 1,
    );
    ctx.outputs.push(compositor_kernel_native_context_render_base::render::OutputPipe {
        crtc: built.crtc,
        mode,
        output,
        damage_tracker,
        drm_output: Some(built.drm_output),
        hdr_caps: built.hdr,
        hdr_active,
        hdr_signalled: false,
        connector: built.connector,
        current_drm_mode: built.drm_mode,
        modes: built.modes,
        mode_revert: None,
        global: Some(global),
        in_flight: false,
    });
    Ok(())
}

/// Dark the PRIMARY pipe (`outputs[0]`) — the always-present anchor kept even when
/// no monitor is connected (the `outputs` non-empty invariant). Mirrors `go_dark`
/// but leaves any secondary pipes to the caller's prune step.
fn go_dark_primary(state: &mut Loop, ctx: &mut NativeRenderContext) {
    ctx.outputs[0].drm_output = None;
    *state.inner.kernel.get_mut(&DISPLAY_OFF_MUT) = true;
}

/// Reconcile (SET reconciler): converge the driven outputs to the set that SHOULD be
/// driven — connected monitors the user has left ACTIVE (settings "Inactive" excludes
/// one). Adds newly-active ones, drops disconnected AND deactivated ones, keeps the
/// primary (`outputs[0]`) as the anchor. If NO connected monitor is active (e.g. a
/// hand-edited prefs file), falls back to driving the first-in-prefs (default) monitor
/// so the compositor is never dark purely from deactivation. Runs at startup, on every
/// udev hotplug, and on an activate/deactivate from settings (`OUTPUT_RECONCILE_REQUEST`).
/// Not user-confirmed — no revert gate. Rebuilds the live cursor-teleport map at the end.
pub fn reconcile(state: &mut Loop, ctx_rc: &Ctx) -> Option<OutputChange> {
    let mut ctx = ctx_rc.borrow_mut();
    let was_dark = ctx.outputs.iter().all(|p| p.drm_output.is_none());
    let connected = {
        let mgr = ctx.drm_output_manager.borrow();
        let drm = mgr.device();
        let res = compositor_kernel_drm_connector_scan_base::scan::resources(drm);
        let infos = compositor_kernel_drm_connector_scan_base::scan::connectors(drm, &res);
        infos.into_iter().filter(|i| i.state() == connector::State::Connected).collect::<Vec<_>>()
    };

    // The set to DRIVE: connected monitors the user has left active. If none are
    // active, fall back to the preferred (first-in-prefs) connected monitor — else the
    // first connected — so we never go dark just because everything was deactivated.
    let profiles = compositor_kernel_graphic_preference_output_profile::profile::get();
    let is_active = |key: &str| {
        profiles.iter().find(|p| p.identity.as_deref() == Some(key)).map(|p| p.active).unwrap_or(true)
    };
    let (drive_set, connected_keys): (Vec<connector::Info>, Vec<String>) = {
        let mgr = ctx.drm_output_manager.borrow();
        let drm = mgr.device();
        let keyed: Vec<(connector::Info, String)> =
            connected.iter().map(|c| (c.clone(), identity_key(drm, c))).collect();
        let connected_keys: Vec<String> = keyed.iter().map(|(_, k)| k.clone()).collect();
        let active: Vec<connector::Info> =
            keyed.iter().filter(|(_, k)| is_active(k)).map(|(c, _)| c.clone()).collect();
        let drive = if !active.is_empty() {
            active
        } else if !connected.is_empty() {
            let chosen = preferred_key()
                .and_then(|pk| keyed.iter().find(|(_, k)| *k == pk).map(|(c, _)| c.clone()))
                .or_else(|| connected.first().cloned());
            chosen.into_iter().collect()
        } else {
            Vec::new()
        };
        (drive, connected_keys)
    };
    let drive_handles: Vec<connector::Handle> = drive_set.iter().map(|c| c.handle()).collect();

    // 1. Prune SECONDARY outputs no longer in the drive set (disconnected OR now
    //    inactive) — keep `outputs[0]` as the anchor (handled in step 2).
    let mut i = 1;
    while i < ctx.outputs.len() {
        if !drive_handles.contains(&ctx.outputs[i].connector) {
            let removed = ctx.outputs.remove(i);
            state.inner.space_state_mut().state.unmap_output(&removed.output);
            // Destroy this output's `wl_output` global so re-adding the monitor doesn't
            // advertise a duplicate (the switch reused ONE Output; add_output makes a
            // fresh one each time, so its global must be torn down here).
            if let Some(gid) = removed.global {
                ctx.display_handle
                    .remove_global::<compositor_support_smithay_dispatch_state_base::state::Dispatch>(gid);
            }
            info!("reconcile: removed output {:?} (disconnected or deactivated)", removed.connector);
            // `removed.drm_output` drops here → frees its CRTC.
        } else {
            i += 1;
        }
    }

    // Whether THIS reconcile brought any pipe up (primary fail-over or a new secondary):
    // a freshly-lit pipe has never flipped, so it needs a forced `All` render to start
    // its own vblank cycle (see the `force_redraw` call at the end).
    let mut brought_up = false;

    // 2. PRIMARY (`outputs[0]`, the never-pruned anchor): if it isn't driving a
    //    drive-set monitor, fail it over to one (preferred first), else go dark.
    let primary_ok = ctx.outputs[0].drm_output.is_some()
        && drive_handles.contains(&ctx.outputs[0].connector);
    if !primary_ok {
        let target = {
            let mgr = ctx.drm_output_manager.borrow();
            pick_target(mgr.device(), &drive_set)
        };
        match target {
            Some(t) => {
                // If the chosen target is ALREADY driven by a secondary pipe (e.g. the
                // user deactivated the anchor's OWN monitor while a secondary stays
                // active), EVICT that secondary first. Otherwise the anchor would build
                // a second pipe on a connector another pipe already drives — a
                // double-drive that fights over the CRTC and blacks the display.
                let th = t.handle();
                let mut i = 1;
                while i < ctx.outputs.len() {
                    if ctx.outputs[i].connector == th && ctx.outputs[i].drm_output.is_some() {
                        let removed = ctx.outputs.remove(i);
                        state.inner.space_state_mut().state.unmap_output(&removed.output);
                        if let Some(gid) = removed.global {
                            ctx.display_handle
                                .remove_global::<compositor_support_smithay_dispatch_state_base::state::Dispatch>(gid);
                        }
                        info!("reconcile: evicted secondary {:?} so the primary can take it over", removed.connector);
                    } else {
                        i += 1;
                    }
                }
                let requested = {
                    let mgr = ctx.drm_output_manager.borrow();
                    pref_mode(mgr.device(), &t)
                };
                // CRTCs still held by surviving secondaries must be excluded so the
                // anchor rebuild can't steal a CRTC one of them is scanning out.
                let busy: Vec<crtc::Handle> = ctx
                    .outputs
                    .iter()
                    .skip(1)
                    .filter(|p| p.drm_output.is_some())
                    .map(|p| p.crtc)
                    .collect();
                if let Err(e) = bring_up(&mut ctx, &t, requested, &busy) {
                    warn!("reconcile primary bring-up failed: {e}; going dark");
                    go_dark_primary(state, &mut ctx);
                } else {
                    *state.inner.kernel.get_mut(&DISPLAY_OFF_MUT) = false;
                    brought_up = true;
                }
            }
            None => {
                go_dark_primary(state, &mut ctx);
                warn!("no active monitor — primary dark, awaiting hotplug/activation");
            }
        }
    }

    // 3. ADD every drive-set monitor not yet driven by any pipe, as an additional
    //    output. Collect first (releases the manager borrow before `add_output`).
    let driven: Vec<connector::Handle> = ctx
        .outputs
        .iter()
        .filter(|p| p.drm_output.is_some())
        .map(|p| p.connector)
        .collect();
    let to_add: Vec<connector::Info> =
        drive_set.iter().filter(|c| !driven.contains(&c.handle())).cloned().collect();
    for c in &to_add {
        let requested = {
            let mgr = ctx.drm_output_manager.borrow();
            pref_mode(mgr.device(), c)
        };
        match add_output(state, &mut ctx, c, requested) {
            Ok(()) => { info!("reconcile: added output {:?}", c.handle()); brought_up = true; }
            Err(e) => warn!("reconcile: add_output failed for {:?}: {e}", c.handle()),
        }
    }

    // 4. Snapshots + teleport map + result.
    let any_live = ctx.outputs.iter().any(|p| p.drm_output.is_some());
    if any_live {
        *state.inner.kernel.get_mut(&DISPLAY_OFF_MUT) = false;
    }
    write_snapshots(state, &ctx);
    drop(ctx);
    // Rebuild the live cursor-teleport map: only active + connected monitors' placements
    // survive (`build_teleport` filters). The active flags come from the live in-memory
    // prefs (settings updates memory + disk together before requesting a reconcile).
    let layout = compositor_orchestration_driver_output_base::base::build_teleport(&state.inner.preference, &connected_keys);
    *state.inner.kernel.get_mut(&compositor_orchestration_driver_output_base::base::TELEPORT_LAYOUT_MUT) = layout;
    *state.inner.kernel.get_mut(&compositor_orchestration_driver_output_base::base::CURSOR_PLACEMENT_MUT) = None;
    // A pipe brought up in this reconcile has never flipped, so the per-CRTC vblank path
    // (`RenderScope::Crtc`) will never render it. FORCE the redraw ping (which runs
    // `execute(RenderScope::All)`) so the new pipe gets its first frame and starts its own
    // vblank cycle — otherwise a plain `schedule_redraw` no-ops while another pipe is
    // mid-flight and the new output stays dark until the next full resume render.
    if brought_up {
        state.force_redraw();
    } else {
        state.schedule_redraw();
    }
    if !any_live {
        Some(OutputChange::WentDark)
    } else if was_dark {
        Some(OutputChange::Recovered)
    } else {
        Some(OutputChange::Changed)
    }
}
